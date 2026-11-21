//! Public-API restart evidence. Persistence is supplied by the host.
mod common;
use ao_idents::Identity;
use ao_mesh::{
    CopyRecord, Header, MeshConfig, MeshEngine, MeshHello, MeshInput, MeshOutput, Packet,
    PacketClass, RestartError, RestartSnapshot, RestartStore, SeenRecord, sign_packet,
};
use ao_network::{AppHello, Channel, Hello, LinkId, LocalTag};
use rand::{SeedableRng, rngs::StdRng};
fn engine() -> MeshEngine<StdRng> {
    let mut config = MeshConfig::default();
    config.presence.enabled = false;
    config.spray.enabled = false;
    MeshEngine::with_config(StdRng::seed_from_u64(29), config)
}
fn up(e: &mut MeshEngine<StdRng>, id: &Identity, n: u64, now: u64) -> Vec<MeshOutput> {
    let hello = Hello {
        proto_min: 1,
        proto_max: 1,
        caps: ao_mesh::hello::CAPS,
        max_frame: 16384,
        tag: LocalTag::new([n as u8; 16]),
        app_hello: AppHello::new(
            MeshHello::sign(id, [0; 8], [0; 4])
                .unwrap()
                .encode()
                .to_vec(),
        )
        .unwrap(),
    };
    e.handle(
        now,
        MeshInput::LinkUp {
            link: LinkId::new(n),
            remote_hello: hello,
        },
    )
    .unwrap()
}
fn receive(e: &mut MeshEngine<StdRng>, own: &Identity, wire: &[u8], now: u64) -> Vec<MeshOutput> {
    e.handle(
        now,
        MeshInput::Data {
            link: LinkId::new(1),
            channel: Channel::Message,
            payload: wire,
            identity: own,
        },
    )
    .unwrap()
}
fn outgoing(out: &[MeshOutput]) -> usize {
    out.iter()
        .filter(|o| matches!(o, MeshOutput::Send { .. } | MeshOutput::Broadcast { .. }))
        .count()
}
fn relay_wire(sender: &Identity) -> Vec<u8> {
    let payload = b"opaque relay data";
    let mut header = Header {
        class: PacketClass::Direct,
        dst: Some([9; 16]),
        msg_id: [0; 16].into(),
        origin: [0; 16],
        created_ms: u64::MAX,
        lifetime_s: 3600,
        payload_len: payload.len() as u64,
        ttl: 8,
        hops: 0,
    };
    let sig = sign_packet(sender, &mut header, payload).unwrap();
    Packet::new(header, sig, payload).unwrap().encode()
}
#[derive(Default)]
struct HostStore(Option<RestartSnapshot>);
impl RestartStore for HostStore {
    type Error = std::convert::Infallible;
    fn save(&mut self, snapshot: &RestartSnapshot) -> Result<(), Self::Error> {
        self.0 = Some(snapshot.clone());
        Ok(())
    }
    fn load(&mut self) -> Result<Option<RestartSnapshot>, Self::Error> {
        Ok(self.0.clone())
    }
}
#[test]
fn restored_engine_never_rerelays_a_live_seen_packet_across_clock_epochs() {
    let sender = Identity::from_seed([1; 32]);
    let own = Identity::from_seed([2; 32]);
    let next = Identity::from_seed([3; 32]);
    let mut before = engine();
    up(&mut before, &sender, 1, 90_000);
    up(&mut before, &next, 2, 90_000);
    let wire = relay_wire(&sender);
    assert_eq!(outgoing(&receive(&mut before, &own, &wire, 90_001)), 1);
    let mut host = HostStore::default();
    host.save(&before.restart_snapshot(90_002, own.node_id()))
        .unwrap();
    let saved = host.load().unwrap().unwrap();
    assert_eq!(saved.seen.len(), 1);
    let mut after = engine();
    after
        .restore_restart(5, own.node_id(), &saved, 1_000)
        .unwrap();
    assert!(after.neighbors().is_empty());
    up(&mut after, &sender, 1, 5);
    up(&mut after, &next, 2, 5);
    assert_eq!(outgoing(&receive(&mut after, &own, &wire, 6)), 0);
    let expiry = 5 + saved.seen[0].remaining_ms - 1_000;
    assert_eq!(outgoing(&receive(&mut after, &own, &wire, expiry)), 1);
}
#[test]
fn delivered_immunity_and_its_signature_survive_restart() {
    let sender = Identity::from_seed([4; 32]);
    let own = Identity::from_seed([5; 32]);
    let mut before = engine();
    up(&mut before, &sender, 1, 1_000);
    let wire = common::sealed(
        &sender,
        &own,
        1_000,
        PacketClass::Direct,
        |_| {},
        b"restart receipt",
    );
    let id = Packet::decode(&wire).unwrap().header.msg_id;
    let out = receive(&mut before, &own, &wire, 1_001);
    assert!(out.iter().any(|o| matches!(o, MeshOutput::Deliver { .. })));
    assert!(
        before
            .restart_snapshot(1_001, own.node_id())
            .immunity
            .iter()
            .any(|r| r.id == id)
    );
    let snapshot = before.restart_snapshot(1_002, own.node_id());
    assert_eq!(snapshot.immunity.len(), 1);
    let mut after = engine();
    after
        .restore_restart(50, own.node_id(), &snapshot, 5_000)
        .unwrap();
    assert!(
        after
            .restart_snapshot(50, own.node_id())
            .immunity
            .iter()
            .any(|r| r.id == id)
    );
    up(&mut after, &sender, 1, 50);
    assert!(
        !receive(&mut after, &own, &wire, 51)
            .iter()
            .any(|o| matches!(o, MeshOutput::Deliver { .. }))
    );
    let mut bad = snapshot.clone();
    bad.immunity[0].receipt[10] ^= 1;
    let mut fresh = engine();
    assert_eq!(
        fresh.restore_restart(0, own.node_id(), &bad, 0),
        Err(RestartError::InvalidRecord)
    );
    // A failed restore is atomic and does not consume the fresh-engine gate.
    fresh
        .restore_restart(0, own.node_id(), &snapshot, 0)
        .unwrap();
}
#[test]
fn spent_and_wait_phase_copy_tickets_never_reset_on_restore() {
    let own = Identity::from_seed([6; 32]);
    let mut snapshot = engine().restart_snapshot(0, own.node_id());
    for copies in [0, 1, 7, 255] {
        snapshot.copies.push(CopyRecord {
            id: [copies; 16].into(),
            destination: [9; 16],
            copies_left: copies,
            remaining_ms: 1000,
        });
    }
    let mut after = engine();
    after
        .restore_restart(9, own.node_id(), &snapshot, 100)
        .unwrap();
    for copies in [0, 1, 7, 255] {
        assert_eq!(after.spray_copies_left([copies; 16].into()), copies);
    }
    assert_eq!(after.restart_snapshot(10, own.node_id()).copies.len(), 4);
    let mut expired = engine();
    expired
        .restore_restart(0, own.node_id(), &snapshot, 1000)
        .unwrap();
    assert!(expired.restart_snapshot(0, own.node_id()).copies.is_empty());
}
#[test]
fn malformed_oversized_cross_account_and_late_restores_are_atomic() {
    let own = Identity::from_seed([7; 32]);
    let mut saved = engine().restart_snapshot(0, own.node_id());
    let mut fresh = engine();
    saved.version = 2;
    assert_eq!(
        fresh.restore_restart(0, own.node_id(), &saved, 0),
        Err(RestartError::Version)
    );
    saved.version = 1;
    assert_eq!(
        fresh.restore_restart(0, Identity::from_seed([8; 32]).node_id(), &saved, 0),
        Err(RestartError::Owner)
    );
    saved.seen = vec![
        SeenRecord {
            id: [1; 16].into(),
            remaining_ms: 100
        };
        4097
    ];
    assert_eq!(
        fresh.restore_restart(0, own.node_id(), &saved, 0),
        Err(RestartError::Capacity)
    );
    saved.seen.truncate(2);
    assert_eq!(
        fresh.restore_restart(0, own.node_id(), &saved, 0),
        Err(RestartError::InvalidRecord)
    );
    saved.seen.truncate(1);
    assert_eq!(
        fresh.restore_restart(u64::MAX, own.node_id(), &saved, 0),
        Err(RestartError::ClockOverflow)
    );
    fresh.restore_restart(0, own.node_id(), &saved, 0).unwrap();
    assert_eq!(
        fresh.restore_restart(0, own.node_id(), &saved, 0),
        Err(RestartError::AlreadyStarted)
    );
    let mut started = engine();
    up(&mut started, &own, 1, 0);
    assert_eq!(
        started.restore_restart(0, own.node_id(), &saved, 0),
        Err(RestartError::AlreadyStarted)
    );
}
