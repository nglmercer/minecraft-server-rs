//! Resource limits shared by request, filesystem, and archive handlers.

use guardian::backup::ArchiveLimits;

/// Default maximum multipart request size. It is deliberately route-scoped in
/// the router so ordinary JSON endpoints keep much smaller limits.
pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum size of one Modrinth plugin or mod download.
pub const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum number of simultaneous browser downloads retained as open files.
pub const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 32;
/// Maximum number of simultaneously upgraded console WebSockets.
pub const DEFAULT_MAX_CONCURRENT_WEBSOCKETS: usize = 64;
/// Maximum JSON body accepted by the API.
pub const MAX_JSON_BODY_BYTES: usize = 64 * 1024;
/// Maximum text written through the editor endpoint.
pub const MAX_EDIT_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Default expansion limit for one server-scoped archive operation.
pub const DEFAULT_MAX_EXTRACTED_BYTES: u64 = 1024 * 1024 * 1024;
/// Default maximum size of one extracted file.
pub const DEFAULT_MAX_EXTRACTED_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Default number of archive members accepted in one extraction.
pub const DEFAULT_MAX_ARCHIVE_ENTRIES: usize = 10_000;
/// Default quota for all files belonging to one server.
pub const DEFAULT_MAX_SERVER_DISK_BYTES: u64 = 50 * 1024 * 1024 * 1024;
/// Default quota for retained compressed backups of one server.
pub const DEFAULT_MAX_BACKUP_DISK_BYTES: u64 = 50 * 1024 * 1024 * 1024;
/// Fallback host-level heap budget used only when the operating system does not
/// report physical memory.  A normal installation derives this from 75% of the
/// detected host memory at startup.
pub const FALLBACK_MAX_SERVER_MEMORY_MB: u32 = 64 * 1024;

/// Leave room for the panel, the OS, and native memory outside each JVM heap.
pub fn host_memory_budget_mb() -> u32 {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let total_mb = system.total_memory() / 1024 / 1024;
    if total_mb == 0 {
        return FALLBACK_MAX_SERVER_MEMORY_MB;
    }

    total_mb
        .saturating_mul(3)
        .saturating_div(4)
        .min(guardian::MAX_SERVER_MEMORY_MB as u64)
        .max(1) as u32
}

/// Limits that apply to one panel installation.
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    pub max_upload_bytes: u64,
    pub max_download_bytes: u64,
    pub max_extracted_bytes: u64,
    pub max_archive_entries: usize,
    pub max_extracted_file_bytes: u64,
    pub max_server_disk_bytes: u64,
    pub max_backup_disk_bytes: u64,
    pub max_server_memory_mb: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
            max_download_bytes: DEFAULT_MAX_DOWNLOAD_BYTES,
            max_extracted_bytes: DEFAULT_MAX_EXTRACTED_BYTES,
            max_archive_entries: DEFAULT_MAX_ARCHIVE_ENTRIES,
            max_extracted_file_bytes: DEFAULT_MAX_EXTRACTED_FILE_BYTES,
            max_server_disk_bytes: DEFAULT_MAX_SERVER_DISK_BYTES,
            max_backup_disk_bytes: DEFAULT_MAX_BACKUP_DISK_BYTES,
            max_server_memory_mb: host_memory_budget_mb(),
        }
    }
}

impl ResourceLimits {
    pub fn validate(self) -> anyhow::Result<()> {
        anyhow::ensure!(self.max_upload_bytes > 0, "max upload must be positive");
        anyhow::ensure!(self.max_download_bytes > 0, "max download must be positive");
        anyhow::ensure!(
            self.max_extracted_bytes > 0,
            "max extracted bytes must be positive"
        );
        anyhow::ensure!(
            self.max_extracted_file_bytes > 0
                && self.max_extracted_file_bytes <= self.max_extracted_bytes,
            "per-file extraction limit must be positive and no larger than total extraction limit"
        );
        anyhow::ensure!(
            self.max_archive_entries > 0,
            "maximum archive entries must be positive"
        );
        anyhow::ensure!(
            self.max_server_disk_bytes > 0,
            "server disk quota must be positive"
        );
        anyhow::ensure!(
            self.max_backup_disk_bytes > 0,
            "backup disk quota must be positive"
        );
        anyhow::ensure!(
            self.max_server_memory_mb > 0,
            "server memory maximum must be positive"
        );
        anyhow::ensure!(
            self.max_server_memory_mb <= guardian::MAX_SERVER_MEMORY_MB,
            "server memory maximum exceeds the hard safety limit"
        );
        Ok(())
    }

    pub fn archive_limits(self) -> ArchiveLimits {
        ArchiveLimits {
            max_entries: self.max_archive_entries,
            max_total_bytes: self.max_extracted_bytes,
            max_file_bytes: self.max_extracted_file_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_positive_and_ordered() {
        let limits = ResourceLimits::default();
        limits.validate().unwrap();
        assert!(limits.max_extracted_file_bytes <= limits.max_extracted_bytes);
    }

    #[test]
    fn impossible_limits_are_rejected() {
        let mut limits = ResourceLimits::default();
        limits.max_extracted_file_bytes = limits.max_extracted_bytes + 1;
        assert!(limits.validate().is_err());
    }
}
