# Restart metadata (MSH-29)

The host stores `RestartSnapshot` through its implementation of `RestartStore`.
The engine performs no persistence, serialization, key storage or clock reads.
Snapshots contain only seen ids, verified receipt envelopes, and remaining copy
tickets. Message bytes and store metadata remain in MessageStore.

## Host sequence

1. Stop feeding inputs while capturing state and MessageStore copy metadata.
2. Call `engine.restart_snapshot(now, identity.node_id())`.
3. Atomically save that DTO together with the host's current store transaction.
4. At launch, authenticate the host-owned stored data and cap vector lengths
   during deserialization; do not deserialize arbitrary unbounded lengths.
5. Create a fresh engine, load the DTO, then call
   `restore_restart(new_now, identity.node_id(), &snapshot, elapsed_ms)` before
   any input. Propagate both storage errors and typed restore errors.
6. Reconnect links normally. Never restore LinkIds, negotiated capabilities,
   ephemeral session state or transport queues.

Version 1 identifies the local owner. Seen and copy records hold remaining
milliseconds; receipt bytes retain their original signatures and are reverified.
`elapsed_ms` is host-measured time since capture, including time before shutdown
and downtime. It is never computed by subtracting different monotonic epochs.
If this duration is unknown, zero conservatively retains saved guards. Repeated
restarts with unknown downtime can retain guards longer; accurate host elapsed
time is required to age them out across launches. The engine provides no durable
or crash-exact guarantee unless the host commits snapshots consistently with
MessageStore. The application owns authentication/encryption of local snapshots.

Restore validates schema, identity, all three vector bounds, unique ids, local
lifetime bounds and receipt envelope size/signature/id before committing any
state. An error leaves the fresh engine unchanged. A restore or any engine input
closes the restore gate. Deadlines use checked addition; overflow is an error.
Expired records are omitted. Active configuration limits are authoritative;
a snapshot larger than smaller new limits is rejected instead of silently losing
dedupe or immunity. Copy lifetimes are conservatively capped at the Direct
24-hour ceiling from ADR-0001.

Zero-ticket tombstones are retained. Duplicate packet/store observations do not
refill a ticket or extend its lifetime. A fresh ticket can be allocated after the
old one expires. Resetting stale `handed_to` LinkIds does not reset remaining
copies; each later transfer still spends that preserved budget. Receipt gossip
can resume on new verified links, and delivered ids remain immune until expiry.
The aoIdents replay guard is a separate recipient mechanism and is not conflated
with forwarding dedupe. Pending, not-yet-forwarded suppression entries may be
reconsidered; packets already recorded as seen are never re-relayed while live.

## Alternatives and bounds

| Approach | Delivery / latency / radio cost | Memory / CPU / phone battery | Determinism and maintenance |
| --- | --- | --- | --- |
| Typed metadata DTO, host persistence (chosen) | No restart-induced re-relay of retained ids; no new radio traffic | Linear in existing bounded state; receipt verification once at restore | No runtime or serialization dependency; host transaction policy explicit |
| Persist the complete engine and live links | Could retain pending timers, but stale link ids cannot identify new sessions | Larger snapshot includes queues and unrelated state | Couples storage to private internals, config and transport lifecycles |

Default upper bounds are 4096 seen records, 512 immunity records and 4096 copy
records, plus each receipt's existing bounded envelope. A snapshot temporarily
copies bounded metadata; it does not copy message payloads or the whole store.
Maps are rebuilt in temporary tables and swapped only after full validation.
Seen records preserve oldest-first eviction order, followed by any slotless ids.

`cargo test --all-features --test restart` covers observable re-relay suppression
across local epochs, signed immunity retention and tampering, spent/wait/spray
counters, downtime expiry, version/owner rejection, count bounds, duplicate ids,
overflow, atomic failure and the fresh-engine gate. Unit regressions also exercise
stale observation refill and zero capacity. These are engine/persistence seam
tests, not a claim that the Android app has implemented the durable transaction.
