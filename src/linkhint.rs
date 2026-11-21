//! Bounded advisory neighbour selection (MSH-25); never link admission.
//!
//! The host supplies current discovery observations and contact beacon hints
//! derived by aoIdents. A hint match is advice, never identity authentication.

use ao_network::{LinkHintPreference, PeerKey};
#[cfg(feature = "adapter")]
use std::collections::BTreeSet;

/// Maximum observations or contact hints accepted in one refresh.
/// Hard ceiling for host-controlled per-refresh CPU and retained peer advice.
pub const MAX_LINK_HINT_INPUTS: usize = 256;

/// Host-observation limits; oversized refreshes are rejected atomically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkHintConfig {
    /// Maximum candidate peers, clamped to [`MAX_LINK_HINT_INPUTS`].
    pub max_candidates: usize,
    /// Maximum current contact hints, clamped to [`MAX_LINK_HINT_INPUTS`].
    pub max_contacts: usize,
}
impl Default for LinkHintConfig {
    fn default() -> Self {
        Self {
            max_candidates: 32,
            max_contacts: 32,
        }
    }
}

/// Current discovery evidence supplied by the host, without keys or NodeIds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkHintCandidate {
    /// Opaque radio peer observed by the transport owner.
    pub peer: PeerKey,
    /// Current beacon hint; absent when a beacon did not decode.
    pub beacon_hint: Option<[u8; 8]>,
    /// Host-observed additional reachability compared with current neighbours.
    /// Set only from current topology evidence, not RSSI or random identifiers.
    pub adds_diversity: bool,
}

/// An invalid refresh leaves all previously applied advice unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkHintError {
    /// Candidate or contact input exceeds its configured or hard limit.
    TooManyInputs,
    /// The candidate list names the same radio peer twice.
    DuplicatePeer,
}
impl std::fmt::Display for LinkHintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyInputs => f.write_str("link hint input bound exceeded"),
            Self::DuplicatePeer => f.write_str("duplicate link hint peer"),
        }
    }
}
impl std::error::Error for LinkHintError {}

#[cfg(feature = "adapter")]
pub(crate) fn validate(
    config: LinkHintConfig,
    candidates: &[LinkHintCandidate],
    contacts: &[[u8; 8]],
) -> Result<(), LinkHintError> {
    if candidates.len() > config.max_candidates.min(MAX_LINK_HINT_INPUTS)
        || contacts.len() > config.max_contacts.min(MAX_LINK_HINT_INPUTS)
    {
        return Err(LinkHintError::TooManyInputs);
    }
    let mut peers = BTreeSet::new();
    for candidate in candidates {
        if !peers.insert(candidate.peer) {
            return Err(LinkHintError::DuplicatePeer);
        }
    }
    Ok(())
}

/// Stateless policy: penalties outrank contact recognition and diversity.
/// The caller obtains `penalized` from the engine's existing budget tracker.
/// Contact hints are current aoIdents pairwise-tag prefixes, never secrets.
/// Only the first [`MAX_LINK_HINT_INPUTS`] contacts are examined; the adapter
/// rejects oversized slices before calling this bounded policy.
#[must_use]
pub fn preference(
    candidate: &LinkHintCandidate,
    contacts: &[[u8; 8]],
    penalized: bool,
) -> LinkHintPreference {
    if penalized {
        LinkHintPreference::Deprioritize
    } else if candidate.adds_diversity
        || candidate.beacon_hint.is_some_and(|hint| {
            contacts
                .iter()
                .take(MAX_LINK_HINT_INPUTS)
                .any(|contact| *contact == hint)
        })
    {
        LinkHintPreference::Prefer
    } else {
        LinkHintPreference::Neutral
    }
}
