# MSH-25: advisory neighbour hints

## Choice

Two options were considered: let mesh reserve/evict radio links, or pass
bounded advice to aoNetwork's existing typed hint API. The former duplicates
admission and can violate radio budgets. We choose advice: aoNetwork alone
owns the degree cap, RSSI floors, cooldown, backoff and slot selection.

The host refreshes current `LinkHintCandidate` observations and current
8-byte contact hints derived with aoIdents `RotatingTag::hint`. Keys and
pairwise secrets never enter mesh. Matching a truncated beacon hint only
suggests a connection; the signed MeshHello remains the identity check.
An absent/invalid beacon cannot match. A host may also mark a candidate as
adding diversity using current reachability evidence. Mesh does not infer
diversity from random tags, signal strength or identity prefixes.

A positive existing mesh budget penalty on a live peer/link binding yields
Deprioritize, taking precedence over contacts and diversity. Otherwise either
contact recognition or diversity yields Prefer; remaining candidates are
Neutral. Disconnected peers have no authenticated mesh penalty binding here;
aoNetwork's abuse/backoff policy remains responsible for reconnect admission.

The adapter must own hint policy for supplied peers. Refresh after discovery,
contact epoch, topology or budget changes. Missing candidates and empty
refreshes remove the adapter's previous advice. A tag rotation with no new
match restores Neutral. Calls reject duplicate peers or oversized inputs
before mutation. Advice is advisory even when network's smaller max_peers
bound evicts a hint. Host discovery evidence must be supplied explicitly:
NetworkEvent does not currently surface TransportEvent::Discovered.

## Bounds and cost

MeshConfig.link_hints defaults to 32 candidates and 32 contact hints, matching
the default mesh neighbour bound. Each limit can be configured down to zero
or up to the hard ceiling of 256. Validation and retained peer identifiers
are bounded by those limits. The adapter retains no contact tags or secrets.
Contact matching is at most candidates * contacts comparisons; penalty
lookup scans bounded live links. Deterministic BTreeMap/BTreeSet ordering
avoids hash/RNG/arrival-order tie breaking. No timer, task, radio packet or
crypto primitive is added. Host refreshes should be event-driven to avoid
polling cost on phones.

## Paired simulator evidence

Command: `cargo test --all-features --test link_hints -- --nocapture`.
Same transport topology, clocks and RNG seed in each pair; only adapter
advice changes. One slot, one in-range contact, equal beacon initiation
hints; pairwise recognition is supplied by the host, as on the real seam.

| Seed | Hints off link time (ms) | Hints on link time (ms) |
| --- | ---: | ---: |
| 295383 | 4280 | 260 |
| 295384 | 4290 | 290 |
| 295385 | 4260 | 250 |
| 295386 | 4300 | 300 |

Within 2 seconds: off 0/4, on 4/4. Mean time: off 4282.5 ms, on 275 ms.
The degree cap is asserted at every simulation step, including a separate
three-node scenario with two preferred peers competing for one slot.
These are link-formation measurements, not claims about payload delivery,
transmission overhead or real-phone battery use. The policy adds no mesh
payload transmissions; delivery ratio and energy remain unmeasured here.
Unit/adapter regressions cover penalty precedence using actual mesh budget
state, diversity, missing evidence, rotation, removal and atomic bounds.

## Compatibility

No mesh packet, hello capability, sync encoding or persisted snapshot changes.
The adapter uses the merged aoNetwork NET-16 API, with no local patch override.
