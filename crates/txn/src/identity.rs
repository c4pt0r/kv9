//! Transaction identity and recovery-decision types.
//!
//! These types deliberately separate a transaction's timestamp from the
//! txn-group/timeline generation that gives the timestamp meaning.  They freeze the
//! contract used by the public API; the authority that issues and validates them is a
//! later runtime increment.

use kv9_common::{KeyspaceId, Result, TimeStamp, TimelineId, TxnGroupId, UserKey};

/// Incarnation of one TSO timeline.
///
/// A timestamp from an earlier generation must never be compared with or committed on
/// a replacement generation, even if both generations reuse the same [`TimelineId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimelineGeneration(pub u64);

/// Complete identity of one transaction within its consistency domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TxnId {
    pub txn_group: TxnGroupId,
    pub timeline: TimelineId,
    pub timeline_generation: TimelineGeneration,
    pub start_ts: TimeStamp,
}

/// A user key qualified by the keyspace whose codec and txn group give it meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedKey {
    pub keyspace: KeyspaceId,
    pub user_key: UserKey,
}

/// Descriptor returned by the (future) server-side begin authority and presented to
/// every participant.
///
/// `keyspace` is intentionally repeated next to the qualified primary.  Decoding must
/// reject disagreement rather than choosing one field and silently routing elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxnDescriptor {
    pub keyspace: KeyspaceId,
    pub id: TxnId,
    pub primary: QualifiedKey,
}

/// Server-internal authority to commit one exact transaction.
///
/// This type is not carried by the public commit request.  The future timeline provider
/// issues it after validating the presented descriptor; the Percolator executor consumes
/// it.  Keeping it distinct prevents a public naked `commit_ts` from reappearing in the
/// Rust API while the provider is still unimplemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitAuthority {
    transaction: TxnId,
    commit_ts: TimeStamp,
}

impl CommitAuthority {
    /// Construct the result returned by a [`TxnAuthorityProvider`].
    ///
    /// This constructor is an in-process provider seam, not a wire decoder.  Public
    /// commit requests never carry this type or a commit timestamp.
    pub fn from_provider(transaction: TxnId, commit_ts: TimeStamp) -> Self {
        Self {
            transaction,
            commit_ts,
        }
    }

    /// Structural validation that can be performed without consulting the provider.
    /// Freshness of `timeline_generation` still requires the provider's durable state.
    pub fn is_for(self, descriptor: &TxnDescriptor) -> bool {
        self.transaction == descriptor.id && self.commit_ts > descriptor.id.start_ts
    }

    pub fn commit_ts(self) -> TimeStamp {
        self.commit_ts
    }
}

/// Authority seam for transaction identity and commit-time issuance.
///
/// A descriptor decoded from the public wire is only a *presented* descriptor.  Before
/// executing a participant operation, the service must ask this provider to validate
/// its group and timeline generation against durable state.  The initial Phase-2
/// increment freezes this contract but deliberately ships no production provider.
pub trait TxnAuthorityProvider: Send + Sync + 'static {
    /// Issue a new descriptor after resolving the qualified primary to its txn group.
    fn begin(&self, primary: QualifiedKey) -> Result<TxnDescriptor>;

    /// Validate a descriptor presented by a client against current durable authority.
    fn validate(&self, presented: &TxnDescriptor) -> Result<()>;

    /// Issue a commit timestamp from the same group and timeline generation.
    fn issue_commit(&self, transaction: &TxnDescriptor) -> Result<CommitAuthority>;
}

/// Durable decision observed at the transaction's qualified primary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnStatus {
    Locked,
    Committed { commit_ts: TimeStamp },
    RolledBack,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::Error;

    fn descriptor() -> TxnDescriptor {
        TxnDescriptor {
            keyspace: KeyspaceId(7),
            id: TxnId {
                txn_group: TxnGroupId(11),
                timeline: TimelineId(13),
                timeline_generation: TimelineGeneration(17),
                start_ts: TimeStamp(19),
            },
            primary: QualifiedKey {
                keyspace: KeyspaceId(7),
                user_key: b"primary".to_vec(),
            },
        }
    }

    #[test]
    fn commit_authority_is_bound_to_the_complete_transaction_identity() {
        let descriptor = descriptor();
        let good = CommitAuthority::from_provider(descriptor.id, TimeStamp(20));
        assert!(good.is_for(&descriptor));

        let mut stale = good;
        stale.transaction.timeline_generation = TimelineGeneration(16);
        assert!(!stale.is_for(&descriptor));

        let mut other_group = good;
        other_group.transaction.txn_group = TxnGroupId(12);
        assert!(!other_group.is_for(&descriptor));

        assert!(
            !CommitAuthority::from_provider(descriptor.id, descriptor.id.start_ts)
                .is_for(&descriptor)
        );
    }

    struct TestProvider {
        current_generation: TimelineGeneration,
    }

    impl TxnAuthorityProvider for TestProvider {
        fn begin(&self, primary: QualifiedKey) -> Result<TxnDescriptor> {
            Ok(TxnDescriptor {
                keyspace: primary.keyspace,
                id: TxnId {
                    txn_group: TxnGroupId(11),
                    timeline: TimelineId(13),
                    timeline_generation: self.current_generation,
                    start_ts: TimeStamp(19),
                },
                primary,
            })
        }

        fn validate(&self, presented: &TxnDescriptor) -> Result<()> {
            if presented.id.timeline_generation != self.current_generation {
                return Err(Error::MetaNotReady(
                    "transaction timeline generation is stale".into(),
                ));
            }
            Ok(())
        }

        fn issue_commit(&self, transaction: &TxnDescriptor) -> Result<CommitAuthority> {
            self.validate(transaction)?;
            Ok(CommitAuthority::from_provider(
                transaction.id,
                TimeStamp(transaction.id.start_ts.0 + 1),
            ))
        }
    }

    #[test]
    fn provider_contract_rejects_a_stale_generation_before_issuing_commit_authority() {
        let provider = TestProvider {
            current_generation: TimelineGeneration(17),
        };
        let issued = provider
            .begin(QualifiedKey {
                keyspace: KeyspaceId(7),
                user_key: b"primary".to_vec(),
            })
            .unwrap();
        assert!(provider.issue_commit(&issued).unwrap().is_for(&issued));

        let mut stale = issued;
        stale.id.timeline_generation = TimelineGeneration(16);
        assert!(matches!(
            provider.issue_commit(&stale),
            Err(Error::MetaNotReady(message))
                if message == "transaction timeline generation is stale"
        ));
    }
}
