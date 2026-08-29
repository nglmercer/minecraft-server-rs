//! Retention policy and engine.

use crate::store::{BackupRetentionPolicy, StoredBackup};

/// Result of retention: which backup IDs should be deleted.
pub fn plan_retention(
    backups: &[StoredBackup],
    policy: &BackupRetentionPolicy,
    newly_created_id: &str,
) -> Vec<String> {
    if backups.is_empty() {
        return Vec::new();
    }
    // Sort newest -> oldest by created_at (RFC3339 is lexicographically sortable, but parse to be safe)
    let mut sorted: Vec<&StoredBackup> = backups.iter().collect();
    sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let mut to_delete = Vec::new();
    let mut remaining: Vec<&StoredBackup> = Vec::new();

    // Protect the newly created backup unconditionally
    for b in sorted {
        if b.id == newly_created_id {
            remaining.push(b);
        } else {
            // Age check: remove if older than max_age_days
            if let Some(max_days) = policy.max_age_days {
                if is_older_than(b, max_days) {
                    to_delete.push(b.id.clone());
                    continue;
                }
            }
            remaining.push(b);
        }
    }

    // After age deletions, enforce max_backups on the remaining set.
    // remaining is newest->oldest, but includes newly created at its position.
    // Re-sort remaining newest->oldest to be deterministic.
    remaining.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if remaining.len() > policy.max_backups {
        // Keep the first max_backups, delete the rest (oldest)
        for b in remaining.iter().skip(policy.max_backups) {
            // Never delete the newly created one, even if it would be beyond limit (shouldn't happen
            // because we already protected it, but be defensive)
            if b.id == newly_created_id {
                continue;
            }
            to_delete.push(b.id.clone());
        }
    }

    // Also ensure we never delete the only backup until replacement succeeded.
    // If total successful backups would become 0 and newly_created is the only one,
    // we must not delete it. Our logic already protects newly_created, so this is satisfied.
    // Additionally, if newly_created failed, this function wouldn't be called, so existing backups remain.

    // Deterministic dedup
    to_delete.sort();
    to_delete.dedup();
    to_delete
}

fn is_older_than(backup: &StoredBackup, max_days: u32) -> bool {
    let Ok(created) = time::OffsetDateTime::parse(
        &backup.created_at,
        &time::format_description::well_known::Rfc3339,
    ) else {
        return false;
    };
    let now = time::OffsetDateTime::now_utc();
    // Use exact duration but truncated to seconds to avoid flakiness from
    // sub-second execution delta between backup creation and retention check.
    // Backup exactly max_days old is kept; > max_days deletes.
    let age_seconds = (now - created).whole_seconds();
    age_seconds > max_days as i64 * 86_400
}

#[allow(dead_code)]
pub fn is_older_than_exact(backup: &StoredBackup, max_days: u32) -> bool {
    is_older_than(backup, max_days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{BackupProviderKind, StoredBackup};

    fn backup(id: &str, age_days: i64) -> StoredBackup {
        let created = time::OffsetDateTime::now_utc() - time::Duration::days(age_days);
        StoredBackup {
            id: id.into(),
            server_id: "srv-1".into(),
            provider: BackupProviderKind::Local,
            remote_id: id.into(),
            created_at: created
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
            size_bytes: 100,
            checksum_sha256: None,
            note: String::new(),
            google_drive_folder_id: None,
            google_drive_credential_ref: None,
        }
    }

    #[test]
    fn default_retains_exactly_one() {
        let policy = BackupRetentionPolicy::default();
        assert_eq!(policy.max_backups, 1);
        assert_eq!(policy.max_age_days, None);
        let a = backup("A", 1);
        let b = backup("B", 0);
        let to_delete = plan_retention(&[a.clone(), b.clone()], &policy, &b.id);
        assert_eq!(to_delete, vec!["A"]);
    }

    #[test]
    fn max_backups_3_keeps_newest_3() {
        let policy = BackupRetentionPolicy {
            max_backups: 3,
            max_age_days: None,
        };
        let a = backup("A", 3);
        let b = backup("B", 2);
        let c = backup("C", 1);
        let d = backup("D", 0);
        let to_delete = plan_retention(
            &[a.clone(), b.clone(), c.clone(), d.clone()],
            &policy,
            &d.id,
        );
        assert_eq!(to_delete, vec!["A"]);
        // If A already gone, B C D remain
        let to_delete2 = plan_retention(&[b.clone(), c.clone(), d.clone()], &policy, &d.id);
        assert!(to_delete2.is_empty());
    }

    #[test]
    fn max_age_days_3_removes_old() {
        let policy = BackupRetentionPolicy {
            max_backups: 10,
            max_age_days: Some(3),
        };
        let a = backup("A", 5);
        let b = backup("B", 3);
        let c = backup("C", 2);
        let d = backup("D", 0);
        let to_delete = plan_retention(
            &[a.clone(), b.clone(), c.clone(), d.clone()],
            &policy,
            &d.id,
        );
        assert_eq!(to_delete, vec!["A"]);
        // B is exactly 3 days old, should be kept (age > max => delete)
        assert!(!to_delete.contains(&"B".to_string()));
    }

    #[test]
    fn combined_limits() {
        let policy = BackupRetentionPolicy {
            max_backups: 3,
            max_age_days: Some(3),
        };
        // A 5d, B 2d, C1d, D0d -> A removed by age, then 3 remain, no count delete
        let a = backup("A", 5);
        let b = backup("B", 2);
        let c = backup("C", 1);
        let d = backup("D", 0);
        let to_delete = plan_retention(
            &[a.clone(), b.clone(), c.clone(), d.clone()],
            &policy,
            &d.id,
        );
        assert_eq!(to_delete, vec!["A"]);
    }

    #[test]
    fn protects_newly_created_backup() {
        let policy = BackupRetentionPolicy {
            max_backups: 1,
            max_age_days: Some(1),
        };
        // Even if new backup is old (shouldn't happen), it is protected
        let old_new = backup("new", 5);
        let old = backup("old", 5);
        let to_delete = plan_retention(&[old.clone(), old_new.clone()], &policy, &old_new.id);
        // old should be deleted, new protected
        assert!(to_delete.contains(&"old".to_string()));
        assert!(!to_delete.contains(&"new".to_string()));
    }

    #[test]
    fn critical_failure_never_deletes_last_good_backup() {
        // Existing A exists, B upload fails => retention not run, so A remains.
        // We simulate retention not being called on failure; plan_retention only called on success.
        // Here we test that if we have A and would create B but B fails, we don't delete A.
        let policy = BackupRetentionPolicy::default();
        let a = backup("A", 1);
        // No B in list because it failed to upload
        let to_delete = plan_retention(std::slice::from_ref(&a), &policy, "B");
        // B not in list, so nothing to delete, A stays
        assert!(to_delete.is_empty());
    }

    #[test]
    fn validation() {
        assert!(BackupRetentionPolicy {
            max_backups: 0,
            max_age_days: None
        }
        .validate()
        .is_err());
        assert!(BackupRetentionPolicy {
            max_backups: 1001,
            max_age_days: None
        }
        .validate()
        .is_err());
        assert!(BackupRetentionPolicy {
            max_backups: 1,
            max_age_days: Some(0)
        }
        .validate()
        .is_err());
        assert!(BackupRetentionPolicy {
            max_backups: 1,
            max_age_days: Some(1)
        }
        .validate()
        .is_ok());
    }
}
