//! Immutable ID-sorted sidecars. Positions always refer to the original Parquet rows.
use crate::{
    graph::{DESTINATION, ID, SOURCE},
    persistence::{crc32, failpoint},
    rows::{column, DeleteVector},
    Error, Result,
};
use datafusion::arrow::record_batch::RecordBatch;
use std::{fs, fs::OpenOptions, io::Write, path::Path};

const MAGIC: &[u8; 8] = b"GFRIDX01";
const HEADER: usize = 24;

#[derive(Debug)]
pub(crate) struct Entry {
    pub id: u64,
    pub position: usize,
    pub source: u64,
    pub destination: u64,
}

pub(crate) fn write(path: &Path, batches: &[RecordBatch], edge: bool) -> Result<u64> {
    let mut entries = Vec::new();
    for batch in batches {
        let ids = column(batch, ID);
        let endpoints = edge.then(|| (column(batch, SOURCE), column(batch, DESTINATION)));
        for row in 0..batch.num_rows() {
            entries.push(Entry {
                id: ids.value(row),
                position: entries.len(),
                source: endpoints.map_or(0, |(source, _)| source.value(row)),
                destination: endpoints.map_or(0, |(_, destination)| destination.value(row)),
            });
        }
    }
    entries.sort_unstable_by_key(|entry| entry.id);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&u64::from(edge).to_le_bytes());
    for entry in entries {
        bytes.extend_from_slice(&entry.id.to_le_bytes());
        bytes.extend_from_slice(&(entry.position as u64).to_le_bytes());
        if edge {
            bytes.extend_from_slice(&entry.source.to_le_bytes());
            bytes.extend_from_slice(&entry.destination.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&crc32(&bytes).to_le_bytes());
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    failpoint("row_index_write");
    file.sync_all()?;
    failpoint("row_index_sync");
    Ok(bytes.len() as u64)
}

pub(crate) fn read(path: &Path, size: u64, rows: usize, edge: bool) -> Result<Vec<Entry>> {
    let corrupt = || Error::Corrupt(format!("invalid row index: {}", path.display()));
    let width = if edge { 32usize } else { 16 };
    let expected = rows
        .checked_mul(width)
        .and_then(|size| size.checked_add(HEADER + 4))
        .ok_or_else(corrupt)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| corrupt())?;
    if !metadata.is_file() || size != expected as u64 || metadata.len() != size {
        return Err(corrupt());
    }
    let bytes = fs::read(path).map_err(|_| corrupt())?;
    if bytes.len() != expected {
        return Err(corrupt());
    }
    let number = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    if &bytes[..8] != MAGIC
        || number(8) != rows as u64
        || number(16) != u64::from(edge)
        || crc32(&bytes[..expected - 4])
            != u32::from_le_bytes(bytes[expected - 4..].try_into().unwrap())
    {
        return Err(corrupt());
    }
    let mut positions = DeleteVector::default();
    let mut entries: Vec<Entry> = Vec::with_capacity(rows);
    for offset in (HEADER..expected - 4).step_by(width) {
        let id = number(offset);
        let position = usize::try_from(number(offset + 8)).map_err(|_| corrupt())?;
        if entries.last().is_some_and(|entry| entry.id >= id)
            || position >= rows
            || !positions.insert(position)
        {
            return Err(corrupt());
        }
        entries.push(Entry {
            id,
            position,
            source: if edge { number(offset + 16) } else { 0 },
            destination: if edge { number(offset + 24) } else { 0 },
        });
    }
    Ok(entries)
}
