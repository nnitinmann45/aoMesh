#![cfg(feature = "sim")]

use ao_mesh::linkhint::{LinkHintCandidate, LinkHintError, MAX_LINK_HINT_INPUTS, preference};
use ao_mesh::{MeshEngine, NetworkAdapter};
use ao_network::{
    AppHello, BeaconFlags, LinkHintPreference as P, LocalTag, Network, NetworkConfig, PeerKey,
    SimConfig, SimNode, SimTransport, SimWorld,
};
use rand::{SeedableRng, rngs::StdRng};
use std::{
    task::{Context, Waker},
    time::Duration,
};

type Adapter = NetworkAdapter<StdRng, SimTransport>;
fn candidate(peer: PeerKey, hint: u8) -> LinkHintCandidate {
    LinkHintCandidate {
        peer,
        beacon_hint: Some([hint; 8]),
        adds_diversity: false,
    }
}
fn adapter(world: &SimWorld, seed: u64) -> (Adapter, PeerKey, ao_network::NodeId) {
    let radio = world.add_node(SimNode::default()).unwrap();
    let (key, id) = (radio.peer_key(), radio.node());
    let config = NetworkConfig::default().with_max_links(1).unwrap();
    let mut net = Network::new(config, radio);
    net.set_local_hello(
        LocalTag::new([seed as u8; 16]),
        AppHello::new(vec![]).unwrap(),
    );
    net.set_beacon(
        ao_idents::RotatingTag::pairwise([42; 32], ao_idents::EPOCH_MIN_MS)
            .unwrap()
            .hint(0),
        [0; 4],
        BeaconFlags {
            accepting: true,
            ..BeaconFlags::default()
        },
    );
    (
        NetworkAdapter::new(MeshEngine::new(StdRng::seed_from_u64(seed)), net),
        key,
        id,
    )
}
#[test]
fn policy_precedence_and_missing_evidence() {
    let mut c = candidate(PeerKey::new([1; 16]), 7);
    assert_eq!(preference(&c, &[[7; 8]], false), P::Prefer);
    assert_eq!(preference(&c, &[[7; 8]], true), P::Deprioritize);
    assert_eq!(preference(&c, &[], false), P::Neutral);
    c.beacon_hint = None;
    assert_eq!(preference(&c, &[[7; 8]], false), P::Neutral);
    c.adds_diversity = true;
    assert_eq!(preference(&c, &[], false), P::Prefer);
    assert_eq!(preference(&c, &[], true), P::Deprioritize);
}
#[test]
fn refresh_is_bounded_atomic_and_clears_rotated_or_missing_hints() {
    let world = SimWorld::new(SimConfig::default());
    let (mut a, _, _) = adapter(&world, 1);
    let c = candidate(PeerKey::new([2; 16]), 7);
    a.refresh_link_hints(&[c], &[[7; 8]]).unwrap();
    assert_eq!(a.network().link_hint(c.peer), P::Prefer);
    assert_eq!(
        a.refresh_link_hints(&[c, c], &[]),
        Err(LinkHintError::DuplicatePeer)
    );
    assert_eq!(a.network().link_hint(c.peer), P::Prefer);
    assert_eq!(
        a.refresh_link_hints(&[], &vec![[0; 8]; MAX_LINK_HINT_INPUTS + 1]),
        Err(LinkHintError::TooManyInputs)
    );
    assert_eq!(
        a.refresh_link_hints(&vec![c; MAX_LINK_HINT_INPUTS + 1], &[]),
        Err(LinkHintError::TooManyInputs)
    );
    assert_eq!(
        a.refresh_link_hints(&[c], &[[0; 8]; 33]),
        Err(LinkHintError::TooManyInputs)
    );
    assert_eq!(a.network().link_hint(c.peer), P::Prefer);
    a.refresh_link_hints(&[c], &[[8; 8]]).unwrap();
    assert_eq!(a.network().link_hint(c.peer), P::Neutral);
    a.refresh_link_hints(&[c], &[[7; 8]]).unwrap();
    a.refresh_link_hints(&[], &[]).unwrap();
    assert_eq!(a.network().link_hint(c.peer), P::Neutral);
    assert!(a.network().link_hints().is_empty());
}
fn contact_time(seed: u64, hints: bool) -> Option<Duration> {
    let world = SimWorld::new(SimConfig {
        seed,
        ..SimConfig::default()
    });
    let (a, _, aid) = adapter(&world, 1);
    let (b, peer, bid) = adapter(&world, 2);
    let mut nodes = [a, b];
    world.set_in_range(aid, bid, true);
    if hints {
        // Same aoIdents pairwise secret/epoch on the host and the contact.
        let tag = ao_idents::RotatingTag::pairwise([42; 32], ao_idents::EPOCH_MIN_MS).unwrap();
        let hint = tag.hint(0);
        nodes[0]
            .refresh_link_hints(
                &[LinkHintCandidate {
                    peer,
                    beacon_hint: Some(hint),
                    adds_diversity: false,
                }],
                &[hint],
            )
            .unwrap();
    }
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..800 {
        for a in &mut nodes {
            while a.network_mut().poll_event(&mut cx, world.now()).is_ready() {}
            assert!(a.network().links().len() <= 1);
        }
        if let Some(link) = nodes[0]
            .network()
            .links()
            .iter()
            .find(|link| link.peer == peer)
        {
            return Some(link.up_at);
        }
        world.advance(Duration::from_millis(10));
    }
    None
}
#[test]
fn paired_sim_contact_links_within_two_seconds_more_often() {
    let mut hits = [0, 0];
    for seed in [0x481d7, 0x481d8, 0x481d9, 0x481da] {
        let off = contact_time(seed, false).expect("baseline eventually links");
        let on = contact_time(seed, true).expect("hinted contact links");
        hits[0] += usize::from(off <= Duration::from_secs(2));
        hits[1] += usize::from(on <= Duration::from_secs(2));
        println!(
            "seed={seed} hints_off_ms={} hints_on_ms={}",
            off.as_millis(),
            on.as_millis()
        );
        assert!(on + Duration::from_secs(3) < off);
    }
    assert!(hits[1] > hits[0]);
    println!(
        "contact linked within 2s: off={}/4 on={}/4; degree cap=1",
        hits[0], hits[1]
    );
}

#[test]
fn live_mesh_penalties_override_contact_and_diversity_advice() {
    let world = SimWorld::new(SimConfig::default());
    let (mut a, _, aid) = adapter(&world, 1);
    let (mut b, peer, bid) = adapter(&world, 2);
    world.set_in_range(aid, bid, true);
    let c = LinkHintCandidate {
        adds_diversity: true,
        ..candidate(peer, 7)
    };
    a.refresh_link_hints(&[c], &[[7; 8]]).unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..200 {
        while a.network_mut().poll_event(&mut cx, world.now()).is_ready() {}
        while b.network_mut().poll_event(&mut cx, world.now()).is_ready() {}
        world.advance(Duration::from_millis(10));
    }
    let link = a.network().links()[0].link;
    let identity = ao_idents::Identity::from_seed([7; 32]);
    let signed = ao_mesh::MeshHello::sign(&identity, [0; 8], [0; 4]).unwrap();
    let hello = ao_network::Hello {
        proto_min: 1,
        proto_max: 1,
        caps: 0,
        max_frame: 16384,
        tag: LocalTag::new([2; 16]),
        app_hello: AppHello::new(signed.encode().to_vec()).unwrap(),
    };
    a.engine_mut()
        .handle(
            2000,
            ao_mesh::MeshInput::LinkUp {
                link,
                remote_hello: hello,
            },
        )
        .unwrap();
    a.engine_mut()
        .handle(
            2001,
            ao_mesh::MeshInput::Data {
                link,
                channel: ao_network::Channel::Message,
                payload: &[0],
                identity: &identity,
            },
        )
        .unwrap();
    assert!(a.engine().budgets().penalty(link) > 0);
    a.refresh_link_hints(&[c], &[[7; 8]]).unwrap();
    assert_eq!(a.network().link_hint(peer), P::Deprioritize);
}

#[test]
fn several_preferred_peers_still_compete_for_one_slot() {
    let world = SimWorld::new(SimConfig::default());
    let (mut a, _, aid) = adapter(&world, 1);
    let (b, bk, bid) = adapter(&world, 2);
    let (c, ck, cid) = adapter(&world, 3);
    world.set_in_range(aid, bid, true);
    world.set_in_range(aid, cid, true);
    a.refresh_link_hints(&[candidate(bk, 7), candidate(ck, 7)], &[[7; 8]])
        .unwrap();
    let mut nodes = [a, b, c];
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..1000 {
        for a in &mut nodes {
            while a.network_mut().poll_event(&mut cx, world.now()).is_ready() {}
            assert!(a.network().links().len() <= 1);
        }
        world.advance(Duration::from_millis(10));
    }
    assert_eq!(nodes[0].network().links().len(), 1);
}

#[test]
fn configured_zero_capacity_rejects_input_but_allows_disable() {
    let world = SimWorld::new(SimConfig::default());
    let (mut a, _, _) = adapter(&world, 1);
    let config = ao_mesh::MeshConfig {
        link_hints: ao_mesh::linkhint::LinkHintConfig {
            max_candidates: 0,
            max_contacts: 0,
        },
        ..ao_mesh::MeshConfig::default()
    };
    a.replace_engine(MeshEngine::with_config(StdRng::seed_from_u64(1), config));
    assert_eq!(
        a.refresh_link_hints(&[candidate(PeerKey::new([2; 16]), 7)], &[]),
        Err(LinkHintError::TooManyInputs)
    );
    assert_eq!(
        a.refresh_link_hints(&[], &[[7; 8]]),
        Err(LinkHintError::TooManyInputs)
    );
    a.refresh_link_hints(&[], &[]).unwrap();
}
