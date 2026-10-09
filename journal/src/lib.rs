//! `driftbrake-journal`: crash-resilient binary Write-Ahead Log (WAL) persistence
//! for [`driftbrake_core::ReconcileHistory`].
//!
//! Provides zero-data-loss durability across process restarts (Docker, Kubernetes,
//! systemd) so that rolling drift windows ($k_s$, $k_{rev}$) are never wiped out
//! by process recycling.
//!
//! # Format
//! - **Header (16 bytes)**: Magic `b"DRFT"`, Version `1`, Flags `0`, Epoch Timestamp `u64`.
//! - **Frames**: Tag (`1` = Pair, `2` = Revert), Length `u32`, CRC32 `u32`, Payload `L` bytes.
//! - **Torn-Write Recovery**: Partial or corrupted trailing bytes from hard power-cuts
//!   are safely truncated to the last valid checksummed record.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use driftbrake_core::{PredictedProfit, RealizedProfit, ReconcileHistory, RevertEvent};
use thiserror::Error;

const MAGIC: [u8; 4] = *b"DRFT";
const CURRENT_VERSION: u16 = 1;
const HEADER_SIZE: usize = 16;
const TAG_PAIR: u8 = 1;
const TAG_REVERT: u8 = 2;

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid journal magic header: expected DRFT")]
    InvalidMagic,
    #[error("Unsupported journal version: {0}")]
    UnsupportedVersion(u16),
    #[error("CRC32 checksum mismatch in record at offset {offset}")]
    ChecksumMismatch { offset: u64 },
    #[error("File is locked by another process: {0}")]
    AlreadyLocked(PathBuf),
}

/// Standard IEEE 802.3 CRC32 implementation (zero external dependency).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if (crc & 1) != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct LockGuard {
    lock_path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// A disk-backed Write-Ahead Log wrapping [`ReconcileHistory`].
pub struct JournaledHistory {
    inner: ReconcileHistory,
    file: File,
    path: PathBuf,
    max_retained_entries: usize,
    _lock: LockGuard,
}

impl JournaledHistory {
    /// Opens an existing journal file, recovers history, or initializes a new one.
    ///
    /// Locks the journal file exclusively to prevent multi-process corruption.
    pub fn open_or_create<P: AsRef<Path>>(
        path: P,
        max_retained_entries: usize,
    ) -> Result<Self, JournalError> {
        let path = path.as_ref().to_path_buf();
        let lock_path = path.with_extension("lock");

        // Acquire exclusive lock file
        let lock_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path);

        match lock_file {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(JournalError::AlreadyLocked(lock_path));
            }
            Err(e) => return Err(JournalError::Io(e)),
        }

        let lock_guard = LockGuard { lock_path };

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        let mut inner = ReconcileHistory::new();
        let file_len = file.metadata()?.len();

        if file_len == 0 {
            // Write initial 16-byte header
            let mut header = [0u8; HEADER_SIZE];
            header[0..4].copy_from_slice(&MAGIC);
            header[4..6].copy_from_slice(&CURRENT_VERSION.to_be_bytes());
            header[6..8].copy_from_slice(&0u16.to_be_bytes()); // flags
            header[8..16].copy_from_slice(&current_epoch_ms().to_be_bytes());
            file.write_all(&header)?;
            file.flush()?;
        } else {
            // Replay existing journal with torn-write recovery
            file.seek(SeekFrom::Start(0))?;
            let mut header = [0u8; HEADER_SIZE];
            file.read_exact(&mut header)?;

            if header[0..4] != MAGIC {
                return Err(JournalError::InvalidMagic);
            }
            let version = u16::from_be_bytes([header[4], header[5]]);
            if version != CURRENT_VERSION {
                return Err(JournalError::UnsupportedVersion(version));
            }

            let mut valid_offset = HEADER_SIZE as u64;

            loop {
                let mut meta = [0u8; 9]; // tag (1) + len (4) + crc (4)
                match file.read_exact(&mut meta) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                        // Clean EOF or trailing incomplete frame: truncate to last valid
                        file.set_len(valid_offset)?;
                        file.seek(SeekFrom::Start(valid_offset))?;
                        break;
                    }
                    Err(e) => return Err(JournalError::Io(e)),
                }

                let tag = meta[0];
                let payload_len = u32::from_be_bytes([meta[1], meta[2], meta[3], meta[4]]) as usize;
                let expected_crc = u32::from_be_bytes([meta[5], meta[6], meta[7], meta[8]]);

                let mut payload = vec![0u8; payload_len];
                match file.read_exact(&mut payload) {
                    Ok(()) => {
                        let actual_crc = crc32(&payload);
                        if actual_crc != expected_crc {
                            // CRC mismatch / corrupt trailing record: truncate and recover
                            file.set_len(valid_offset)?;
                            file.seek(SeekFrom::Start(valid_offset))?;
                            break;
                        }

                        // Parse valid payload
                        match tag {
                            TAG_PAIR if payload_len == 40 => {
                                let mut pred_bytes = [0u8; 16];
                                pred_bytes.copy_from_slice(&payload[0..16]);
                                let predicted = PredictedProfit(i128::from_be_bytes(pred_bytes));

                                let mut real_bytes = [0u8; 16];
                                real_bytes.copy_from_slice(&payload[16..32]);
                                let realized = RealizedProfit(i128::from_be_bytes(real_bytes));

                                inner.append(predicted, realized);
                                valid_offset = file.stream_position()?;
                            }
                            TAG_REVERT if payload_len >= 74 => {
                                let mut tx_hash = [0u8; 32];
                                tx_hash.copy_from_slice(&payload[0..32]);

                                let mut blk_bytes = [0u8; 8];
                                blk_bytes.copy_from_slice(&payload[32..40]);
                                let block_number = u64::from_be_bytes(blk_bytes);

                                let mut gas_bytes = [0u8; 8];
                                gas_bytes.copy_from_slice(&payload[40..48]);
                                let gas_used = u64::from_be_bytes(gas_bytes);

                                let mut price_bytes = [0u8; 16];
                                price_bytes.copy_from_slice(&payload[48..64]);
                                let effective_gas_price = u128::from_be_bytes(price_bytes);

                                let mut reason_len_bytes = [0u8; 2];
                                reason_len_bytes.copy_from_slice(&payload[72..74]);
                                let reason_len = u16::from_be_bytes(reason_len_bytes) as usize;

                                let reason = if reason_len > 0 && payload_len >= 74 + reason_len {
                                    String::from_utf8(payload[74..74 + reason_len].to_vec()).ok()
                                } else {
                                    None
                                };

                                inner.record_revert(RevertEvent {
                                    tx_hash,
                                    block_number,
                                    reason,
                                    gas_used,
                                    effective_gas_price,
                                });
                                valid_offset = file.stream_position()?;
                            }
                            _ => {
                                // Unknown tag or malformed payload: truncate to last valid
                                file.set_len(valid_offset)?;
                                file.seek(SeekFrom::Start(valid_offset))?;
                                break;
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                        // Torn frame payload: truncate to last valid
                        file.set_len(valid_offset)?;
                        file.seek(SeekFrom::Start(valid_offset))?;
                        break;
                    }
                    Err(e) => return Err(JournalError::Io(e)),
                }
            }
        }

        // Position file pointer at end for subsequent appends
        file.seek(SeekFrom::End(0))?;

        Ok(Self {
            inner,
            file,
            path,
            max_retained_entries,
            _lock: lock_guard,
        })
    }

    /// Append a confirmed pair to in-memory history and write to disk immediately.
    pub fn append(
        &mut self,
        predicted: PredictedProfit,
        realized: RealizedProfit,
    ) -> Result<(), JournalError> {
        let timestamp_ms = current_epoch_ms();
        let mut payload = [0u8; 40];
        payload[0..16].copy_from_slice(&predicted.0.to_be_bytes());
        payload[16..32].copy_from_slice(&realized.0.to_be_bytes());
        payload[32..40].copy_from_slice(&timestamp_ms.to_be_bytes());

        let crc = crc32(&payload);

        let mut frame_header = [0u8; 9];
        frame_header[0] = TAG_PAIR;
        frame_header[1..5].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        frame_header[5..9].copy_from_slice(&crc.to_be_bytes());

        self.file.write_all(&frame_header)?;
        self.file.write_all(&payload)?;
        self.file.flush()?;

        self.inner.append(predicted, realized);
        Ok(())
    }

    /// Record a confirmed-but-reverted transaction and write to disk immediately.
    pub fn record_revert(&mut self, event: RevertEvent) -> Result<(), JournalError> {
        let timestamp_ms = current_epoch_ms();
        let reason_bytes = event.reason.as_deref().unwrap_or("").as_bytes();
        let reason_len = reason_bytes.len().min(u16::MAX as usize) as u16;

        let mut payload = Vec::with_capacity(74 + reason_len as usize);
        payload.extend_from_slice(&event.tx_hash);
        payload.extend_from_slice(&event.block_number.to_be_bytes());
        payload.extend_from_slice(&event.gas_used.to_be_bytes());
        payload.extend_from_slice(&event.effective_gas_price.to_be_bytes());
        payload.extend_from_slice(&timestamp_ms.to_be_bytes());
        payload.extend_from_slice(&reason_len.to_be_bytes());
        payload.extend_from_slice(&reason_bytes[..reason_len as usize]);

        let crc = crc32(&payload);

        let mut frame_header = [0u8; 9];
        frame_header[0] = TAG_REVERT;
        frame_header[1..5].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        frame_header[5..9].copy_from_slice(&crc.to_be_bytes());

        self.file.write_all(&frame_header)?;
        self.file.write_all(&payload)?;
        self.file.flush()?;

        self.inner.record_revert(event);
        Ok(())
    }

    /// Borrows the reconstructed in-memory [`ReconcileHistory`] for policy evaluation.
    pub fn history(&self) -> &ReconcileHistory {
        &self.inner
    }

    /// Compacts the on-disk journal if entries exceed `max_retained_entries`.
    pub fn compact(&mut self) -> Result<(), JournalError> {
        if self.inner.timeline.len() <= self.max_retained_entries {
            return Ok(());
        }

        let temp_path = self.path.with_extension("compact");
        let mut temp_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;

        let mut header = [0u8; HEADER_SIZE];
        header[0..4].copy_from_slice(&MAGIC);
        header[4..6].copy_from_slice(&CURRENT_VERSION.to_be_bytes());
        header[6..8].copy_from_slice(&0u16.to_be_bytes());
        header[8..16].copy_from_slice(&current_epoch_ms().to_be_bytes());
        temp_file.write_all(&header)?;

        let skip = self.inner.timeline.len().saturating_sub(self.max_retained_entries);
        for entry in self.inner.timeline.iter().skip(skip) {
            match entry {
                driftbrake_core::HistoryEntry::ConfirmedPair {
                    predicted,
                    realized,
                } => {
                    let mut payload = [0u8; 40];
                    payload[0..16].copy_from_slice(&predicted.0.to_be_bytes());
                    payload[16..32].copy_from_slice(&realized.0.to_be_bytes());
                    payload[32..40].copy_from_slice(&0u64.to_be_bytes());

                    let crc = crc32(&payload);
                    let mut fh = [0u8; 9];
                    fh[0] = TAG_PAIR;
                    fh[1..5].copy_from_slice(&(payload.len() as u32).to_be_bytes());
                    fh[5..9].copy_from_slice(&crc.to_be_bytes());
                    temp_file.write_all(&fh)?;
                    temp_file.write_all(&payload)?;
                }
                driftbrake_core::HistoryEntry::RevertedTx { event } => {
                    let reason_bytes = event.reason.as_deref().unwrap_or("").as_bytes();
                    let reason_len = reason_bytes.len().min(u16::MAX as usize) as u16;

                    let mut payload = Vec::with_capacity(74 + reason_len as usize);
                    payload.extend_from_slice(&event.tx_hash);
                    payload.extend_from_slice(&event.block_number.to_be_bytes());
                    payload.extend_from_slice(&event.gas_used.to_be_bytes());
                    payload.extend_from_slice(&event.effective_gas_price.to_be_bytes());
                    payload.extend_from_slice(&0u64.to_be_bytes());
                    payload.extend_from_slice(&reason_len.to_be_bytes());
                    payload.extend_from_slice(&reason_bytes[..reason_len as usize]);

                    let crc = crc32(&payload);
                    let mut fh = [0u8; 9];
                    fh[0] = TAG_REVERT;
                    fh[1..5].copy_from_slice(&(payload.len() as u32).to_be_bytes());
                    fh[5..9].copy_from_slice(&crc.to_be_bytes());
                    temp_file.write_all(&fh)?;
                    temp_file.write_all(&payload)?;
                }
            }
        }

        temp_file.flush()?;
        drop(temp_file);

        // Replace original journal with compacted file
        std::fs::rename(&temp_path, &self.path)?;
        self.file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)?;
        self.file.seek(SeekFrom::End(0))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_journal_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("driftbrake_test_{}.dbjournal", nanos))
    }

    #[test]
    fn clean_shutdown_recovers_all_entries() {
        let path = temp_journal_path();

        {
            let mut j = JournaledHistory::open_or_create(&path, 1000).unwrap();
            for i in 1..=20 {
                j.append(PredictedProfit(100), RealizedProfit(i * 5))
                    .unwrap();
            }
            j.record_revert(RevertEvent {
                tx_hash: [7u8; 32],
                block_number: 55,
                reason: Some("revert".into()),
                gas_used: 21_000,
                effective_gas_price: 1_000,
            })
            .unwrap();
        }

        // Reopen in a new session
        let j2 = JournaledHistory::open_or_create(&path, 1000).unwrap();
        assert_eq!(j2.history().pairs.len(), 20);
        assert_eq!(j2.history().reverts.len(), 1);
        assert_eq!(j2.history().timeline.len(), 21);
        assert_eq!(j2.history().consecutive_reverts(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn recovers_cleanly_from_torn_trailing_bytes() {
        let path = temp_journal_path();

        {
            let mut j = JournaledHistory::open_or_create(&path, 1000).unwrap();
            j.append(PredictedProfit(100), RealizedProfit(90)).unwrap();
            j.append(PredictedProfit(100), RealizedProfit(80)).unwrap();
        }

        // Simulate torn write: append 7 corrupt trailing bytes to file
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03])
                .unwrap();
            f.flush().unwrap();
        }

        // Reopening must recover the 2 valid records and truncate corrupt tail
        let j = JournaledHistory::open_or_create(&path, 1000).unwrap();
        assert_eq!(j.history().pairs.len(), 2);
        assert_eq!(j.history().recent_ratios(2), vec![0.9, 0.8]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prevents_concurrent_process_locks() {
        let path = temp_journal_path();
        let _j1 = JournaledHistory::open_or_create(&path, 1000).unwrap();

        // Attempting to open second instance on same path while j1 is alive must fail
        let res = JournaledHistory::open_or_create(&path, 1000);
        assert!(matches!(res, Err(JournalError::AlreadyLocked(_))));

        drop(_j1);
        // After drop, lock is released and open succeeds
        let _j2 = JournaledHistory::open_or_create(&path, 1000).unwrap();

        let _ = std::fs::remove_file(&path);
    }
}
