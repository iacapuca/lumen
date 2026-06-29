//! `lumen-artifact` — the `.lumen` Dashboard Execution Plan container.
//!
//! A single packed file = one content hash = one CDN object = one atomic fetch.
//! Layout:
//!
//! ```text
//! ┌── header (64 bytes, fixed) ─────────────────────────────┐
//! │ magic [u8;4]="LMN1" · container_ver u16 · flags u16      │
//! │ toc_offset u64 · toc_len u32 · section_count u16         │
//! │ _reserved [u8;10] · content_hash [u8;32] (blake3)        │
//! └─────────────────────────────────────────────────────────┘
//! │ sections (blobs, back to back)                           │
//! │ TOC (JSON array of {name, codec, offset, len, hash})     │
//! ```
//!
//! The TOC is written last so the writer can stream sections then backfill
//! offsets. `content_hash` covers everything after the 64-byte header (sections
//! + TOC), so the header is never self-referential. Reads are O(header + TOC);
//! sections are zero-copy slices into the owned byte buffer — no layout parsing
//! is needed to plan queries.

use lumen_shared::{section, Charts, Layout, Manifest, Queries};
use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 4] = *b"LMN1";
pub const CONTAINER_VER: u16 = 1;
pub const HEADER_LEN: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("not a .lumen file (bad magic)")]
    BadMagic,
    #[error("truncated file: {0}")]
    Truncated(&'static str),
    #[error("unsupported container version {0} (this build supports {CONTAINER_VER})")]
    UnsupportedVersion(u16),
    #[error("section not found: {0}")]
    MissingSection(String),
    #[error("content hash mismatch (corrupt artifact)")]
    HashMismatch,
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, ArtifactError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    Json,
    MsgPack,
    Raw,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TocEntry {
    pub name: String,
    pub codec: Codec,
    pub offset: u64,
    pub len: u32,
    /// blake3 of this section's bytes (per-section integrity).
    pub hash: [u8; 32],
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Builds a `.lumen` file from named sections.
#[derive(Default)]
pub struct DepWriter {
    sections: Vec<(String, Codec, Vec<u8>)>,
}

impl DepWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a section encoded as canonical JSON.
    pub fn add_json<T: Serialize>(&mut self, name: &str, value: &T) -> Result<&mut Self> {
        let bytes = serde_json::to_vec(value)?;
        self.sections.push((name.to_owned(), Codec::Json, bytes));
        Ok(self)
    }

    /// Add a raw byte section (e.g. `theme.css`).
    pub fn add_raw(&mut self, name: &str, bytes: Vec<u8>) -> &mut Self {
        self.sections.push((name.to_owned(), Codec::Raw, bytes));
        self
    }

    /// Serialize to the final `.lumen` byte buffer.
    pub fn finish(self) -> Vec<u8> {
        // 1. Lay sections out starting at HEADER_LEN.
        let mut body = Vec::new();
        let mut toc: Vec<TocEntry> = Vec::with_capacity(self.sections.len());
        for (name, codec, bytes) in &self.sections {
            let offset = (HEADER_LEN + body.len()) as u64;
            toc.push(TocEntry {
                name: name.clone(),
                codec: *codec,
                offset,
                len: bytes.len() as u32,
                hash: *blake3::hash(bytes).as_bytes(),
            });
            body.extend_from_slice(bytes);
        }

        // 2. Append the TOC after the sections.
        let toc_bytes = serde_json::to_vec(&toc).expect("toc serializes");
        let toc_offset = (HEADER_LEN + body.len()) as u64;
        let toc_len = toc_bytes.len() as u32;
        body.extend_from_slice(&toc_bytes);

        // 3. content_hash over the whole body (sections + TOC).
        let content_hash = *blake3::hash(&body).as_bytes();

        // 4. Header.
        let mut out = Vec::with_capacity(HEADER_LEN + body.len());
        out.extend_from_slice(&MAGIC); // 0..4
        out.extend_from_slice(&CONTAINER_VER.to_le_bytes()); // 4..6
        out.extend_from_slice(&0u16.to_le_bytes()); // 6..8 flags
        out.extend_from_slice(&toc_offset.to_le_bytes()); // 8..16
        out.extend_from_slice(&toc_len.to_le_bytes()); // 16..20
        out.extend_from_slice(&(self.sections.len() as u16).to_le_bytes()); // 20..22
        out.extend_from_slice(&[0u8; 10]); // 22..32 reserved
        out.extend_from_slice(&content_hash); // 32..64
        debug_assert_eq!(out.len(), HEADER_LEN);

        out.extend_from_slice(&body);
        out
    }
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// A loaded DEP. Owns its bytes; sections are slices into them.
pub struct Dep {
    bytes: Vec<u8>,
    toc: Vec<TocEntry>,
    content_hash: [u8; 32],
}

impl Dep {
    /// Parse + validate a `.lumen` byte buffer (magic, version, content hash).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(ArtifactError::Truncated("header"));
        }
        if bytes[0..4] != MAGIC {
            return Err(ArtifactError::BadMagic);
        }
        let ver = u16::from_le_bytes([bytes[4], bytes[5]]);
        if ver != CONTAINER_VER {
            return Err(ArtifactError::UnsupportedVersion(ver));
        }
        let toc_offset = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
        let toc_len = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        let mut content_hash = [0u8; 32];
        content_hash.copy_from_slice(&bytes[32..64]);

        // checked_add: a crafted offset/len must not overflow usize or run past EOF.
        let toc_end = toc_offset
            .checked_add(toc_len)
            .filter(|end| *end <= bytes.len())
            .ok_or(ArtifactError::Truncated("toc"))?;

        // Verify content hash over the body (everything after the header).
        let actual = *blake3::hash(&bytes[HEADER_LEN..]).as_bytes();
        if actual != content_hash {
            return Err(ArtifactError::HashMismatch);
        }

        let toc: Vec<TocEntry> = serde_json::from_slice(&bytes[toc_offset..toc_end])?;
        Ok(Dep {
            bytes,
            toc,
            content_hash,
        })
    }

    pub fn content_hash_hex(&self) -> String {
        hex::encode(self.content_hash)
    }

    /// `"blake3:<64hex>"` — the value used for the manifest, ETag, CDN key.
    pub fn content_hash_tagged(&self) -> String {
        format!("blake3:{}", self.content_hash_hex())
    }

    /// Raw bytes of a section by name.
    pub fn section(&self, name: &str) -> Option<&[u8]> {
        let e = self.toc.iter().find(|e| e.name == name)?;
        let start = e.offset as usize;
        let end = start.checked_add(e.len as usize)?;
        self.bytes.get(start..end)
    }

    fn json<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Result<T> {
        let raw = self
            .section(name)
            .ok_or_else(|| ArtifactError::MissingSection(name.to_owned()))?;
        Ok(serde_json::from_slice(raw)?)
    }

    // Typed accessors over the shared DEP model.
    pub fn manifest(&self) -> Result<Manifest> {
        self.json(section::MANIFEST)
    }
    pub fn layout(&self) -> Result<Layout> {
        self.json(section::LAYOUT)
    }
    pub fn queries(&self) -> Result<Queries> {
        self.json(section::QUERIES)
    }
    pub fn charts(&self) -> Result<Charts> {
        self.json(section::CHARTS)
    }
    pub fn theme_css(&self) -> &[u8] {
        self.section(section::THEME).unwrap_or(&[])
    }

    /// Names of all sections present (TOC-driven; unknown sections are tolerated).
    pub fn section_names(&self) -> Vec<&str> {
        self.toc.iter().map(|e| e.name.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_hash_verify() {
        let mut w = DepWriter::new();
        w.add_json("manifest", &serde_json::json!({"hello":"world"}))
            .unwrap();
        w.add_raw("theme", b"body{}".to_vec());
        let bytes = w.finish();

        let dep = Dep::from_bytes(bytes).unwrap();
        assert_eq!(dep.theme_css(), b"body{}");
        assert!(dep.section_names().contains(&"manifest"));
        assert_eq!(dep.content_hash_hex().len(), 64);
    }

    #[test]
    fn corruption_is_detected() {
        let mut w = DepWriter::new();
        w.add_raw("theme", b"body{}".to_vec());
        let mut bytes = w.finish();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff; // flip a byte in the body
        assert!(matches!(
            Dep::from_bytes(bytes),
            Err(ArtifactError::HashMismatch)
        ));
    }

    #[test]
    fn rejects_non_lumen() {
        assert!(matches!(
            Dep::from_bytes(vec![0u8; 128]),
            Err(ArtifactError::BadMagic)
        ));
    }
}
