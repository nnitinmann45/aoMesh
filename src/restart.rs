//! Host-owned snapshot storage for mesh dedupe, immunity and spray tickets.
//!
//! No filesystem, database, key or wall clock enters the engine. The host
//! serializes this versioned DTO, binds it to its identity, and atomically
//! stores it with MessageStore metadata. All durations are remaining local
//! lifetimes, so a new monotonic clock epoch can safely restore them.
use crate::MessageId;
use ao_idents::NodeId;

/// Snapshot schema version. This is storage metadata, never a radio format.
pub const RESTART_VERSION: u8 = 1;
/// Maximum retained Direct lifetime, matching ADR-0001.
pub const MAX_RESTART_LIFETIME_MS: u64 = 86_400_000;
/// An already-seen content address, in oldest-first recording order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenRecord {
    /// Content address.
    pub id: MessageId,
    /// Local lifetime remaining at capture time.
    pub remaining_ms: u64,
}
/// A verified delivery receipt that keeps a message immune from re-sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmunityRecord {
    /// Delivered message id, checked against the signed receipt on restore.
    pub id: MessageId,
    /// Complete signed receipt envelope; bounded and reverified on restore.
    pub receipt: Vec<u8>,
    /// Local lifetime remaining at capture time.
    pub remaining_ms: u64,
}
/// A retained spray ticket. Zero is meaningful: all local copies were spent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyRecord {
    /// Content address.
    pub id: MessageId,
    /// Opaque destination identity.
    pub destination: [u8; 16],
    /// Remaining ticket, including zero and wait-phase one.
    pub copies_left: u8,
    /// Local lifetime remaining at capture time.
    pub remaining_ms: u64,
}
/// Versioned mesh-owned metadata only; message bytes remain in MessageStore.
///
/// The host must cap lengths during deserialization too. Restore validates all
/// record counts before rebuilding tables and commits the new state atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartSnapshot {
    /// Storage schema version.
    pub version: u8,
    /// Account that owns this local state, never a secret.
    pub owner: NodeId,
    /// Bounded forwarding dedupe records, oldest first.
    pub seen: Vec<SeenRecord>,
    /// Bounded signed immunity receipts.
    pub immunity: Vec<ImmunityRecord>,
    /// Bounded local copy tickets, not packet payloads.
    pub copies: Vec<CopyRecord>,
}
/// Storage integration seam. The application implements persistence and errors.
///
/// Save the snapshot atomically with the host's MessageStore copy metadata.
/// Call restore on a fresh engine before feeding any link, timer or packet.
pub trait RestartStore {
    /// Host-defined storage error.
    type Error;
    /// Load the last complete snapshot, or None on first launch.
    fn load(&mut self) -> Result<Option<RestartSnapshot>, Self::Error>;
    /// Commit the snapshot durably; never acknowledge an incomplete write.
    fn save(&mut self, snapshot: &RestartSnapshot) -> Result<(), Self::Error>;
}
/// Restore failure leaves the fresh engine unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartError {
    /// This storage version is not understood.
    Version,
    /// State belongs to another local identity.
    Owner,
    /// Restore must precede all engine inputs and can run only once.
    AlreadyStarted,
    /// A vector or receipt exceeds the active configuration's bounds.
    Capacity,
    /// Duplicate ids, invalid tickets/lifetimes or a forged receipt.
    InvalidRecord,
    /// A new local deadline cannot be represented at this clock value.
    ClockOverflow,
}
impl core::fmt::Display for RestartError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "restart rejected: {self:?}")
    }
}
impl std::error::Error for RestartError {}

pub(crate) fn deadline(
    now: u64,
    remaining: u64,
    elapsed: u64,
    ceiling: u64,
) -> Result<Option<u64>, RestartError> {
    if remaining > ceiling {
        return Err(RestartError::InvalidRecord);
    }
    let remaining = remaining.saturating_sub(elapsed);
    if remaining == 0 {
        return Ok(None);
    }
    now.checked_add(remaining)
        .map(Some)
        .ok_or(RestartError::ClockOverflow)
}
