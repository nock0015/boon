//! Versioned ownership-safe bridge. Only immutable Python-owned byte chunks cross
//! extension modules; no Rust pointers, layouts or allocator ownership cross ABI.
use super::{EvidenceOutput, EvidenceRecord};
use boon_parser::FieldValue;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{self, Write},
};

pub(super) const MAGIC: &[u8; 8] = b"BOONEV1\0";
const MAX_FRAME: usize = 16 * 1024 * 1024;
pub(super) struct WireWriter<W> {
    output: W,
    digest: Sha256,
    classes: HashMap<i32, HashMap<String, u32>>,
    next_class: u32,
    fields: HashMap<(u32, u64), u32>,
    next_field: u32,
}
impl<W: Write> WireWriter<W> {
    pub(super) fn new(output: W) -> Self {
        Self {
            output,
            digest: Sha256::new(),
            classes: HashMap::new(),
            next_class: 0,
            fields: HashMap::new(),
            next_field: 0,
        }
    }
    fn bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.output.write_all(bytes)?;
        self.digest.update(bytes);
        Ok(())
    }
    fn frame(&mut self, payload: &[u8]) -> io::Result<()> {
        if payload.len() > MAX_FRAME {
            return Err(io::Error::other("Evidence wire frame exceeds 16 MiB"));
        }
        self.bytes(&(payload.len() as u32).to_le_bytes())?;
        self.bytes(payload)
    }
    fn class(&mut self, source_class: i32, name: &str) -> io::Result<u32> {
        if let Some(id) = self
            .classes
            .get(&source_class)
            .and_then(|names| names.get(name))
        {
            return Ok(*id);
        }
        let id = self.next_class;
        self.next_class = self
            .next_class
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Evidence wire class schema overflow"))?;
        let mut payload = vec![2];
        u32v(&mut payload, id);
        text(&mut payload, name)?;
        self.frame(&payload)?;
        self.classes
            .entry(source_class)
            .or_default()
            .insert(name.into(), id);
        Ok(id)
    }
    fn field(&mut self, class: u32, key: u64, name: Option<&str>) -> io::Result<u32> {
        if let Some(id) = self.fields.get(&(class, key)) {
            return Ok(*id);
        }
        let id = self.next_field;
        self.next_field = self
            .next_field
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Evidence wire schema overflow"))?;
        let mut payload = vec![3];
        u32v(&mut payload, id);
        u32v(&mut payload, class);
        u64v(&mut payload, key);
        match name {
            None => payload.push(0),
            Some(name) => {
                payload.push(1);
                text(&mut payload, name)?;
            }
        }
        self.frame(&payload)?;
        self.fields.insert((class, key), id);
        Ok(id)
    }
}
fn u32v(out: &mut Vec<u8>, v: u32) {
    out.extend(v.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, v: u64) {
    out.extend(v.to_le_bytes());
}
fn blob(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    u32v(out, u32::try_from(bytes.len()).map_err(io::Error::other)?);
    out.extend(bytes);
    Ok(())
}
fn text(out: &mut Vec<u8>, s: &str) -> io::Result<()> {
    blob(out, s.as_bytes())
}
fn value(out: &mut Vec<u8>, v: &FieldValue) -> io::Result<()> {
    match v {
        FieldValue::Bool(v) => out.extend([1, u8::from(*v)]),
        FieldValue::I32(v) => {
            out.push(2);
            out.extend(v.to_le_bytes());
        }
        FieldValue::I64(v) => {
            out.push(3);
            out.extend(v.to_le_bytes());
        }
        FieldValue::U32(v) => {
            out.push(4);
            u32v(out, *v);
        }
        FieldValue::U64(v) => {
            out.push(5);
            u64v(out, *v);
        }
        FieldValue::F32(v) => {
            out.push(6);
            u32v(out, v.to_bits());
        }
        FieldValue::String(v) => {
            out.push(7);
            blob(out, v)?;
        }
        FieldValue::Vector2(v) => floats(out, 8, v)?,
        FieldValue::Vector3(v) => floats(out, 9, v)?,
        FieldValue::Vector4(v) => floats(out, 10, v)?,
        FieldValue::FloatVector(v) => floats(out, 11, v)?,
        FieldValue::QAngle(v) => floats(out, 12, v)?,
        _ => out.push(0),
    };
    Ok(())
}
fn floats(out: &mut Vec<u8>, tag: u8, values: &[f32]) -> io::Result<()> {
    out.push(tag);
    u32v(out, u32::try_from(values.len()).map_err(io::Error::other)?);
    for v in values {
        u32v(out, v.to_bits());
    }
    Ok(())
}
impl<W: Write> EvidenceOutput for WireWriter<W> {
    fn header(&mut self, header: &serde_json::Value) -> io::Result<()> {
        self.bytes(MAGIC)?;
        let mut payload = vec![1];
        payload.extend(serde_json::to_vec(header)?);
        self.frame(&payload)
    }
    fn entity(&mut self, row: EvidenceRecord<'_>) -> io::Result<()> {
        let class = self.class(row.class_id, row.class_name)?;
        let mut ids = Vec::with_capacity(row.fields.len());
        for (_, field) in &row.fields {
            ids.push(self.field(class, field.key, field.name.as_deref())?);
        }
        let mut payload = vec![
            4,
            match row.kind {
                "state" => 0,
                "delta" => 1,
                _ => 2,
            },
        ];
        u64v(&mut payload, row.sequence);
        payload.extend(row.tick.to_le_bytes());
        payload.extend(row.index.to_le_bytes());
        u32v(&mut payload, row.serial);
        u32v(&mut payload, class);
        payload.push(u8::from(row.active));
        payload.push(u8::from(row.deleted));
        match row.source_game_time {
            None => payload.push(0),
            Some(None) => payload.push(1),
            Some(Some(time)) => {
                payload.push(2);
                u64v(&mut payload, time.to_bits());
            }
        }
        // Provenance is small policy metadata, not field payload. Keep its exact
        // JSON shape until the policy itself receives a new explicit version.
        match row.samples {
            None => u32v(&mut payload, 0),
            Some(samples) => blob(&mut payload, &serde_json::to_vec(&samples)?)?,
        }
        u32v(
            &mut payload,
            u32::try_from(row.fields.len()).map_err(io::Error::other)?,
        );
        for ((_, field), id) in row.fields.into_iter().zip(ids) {
            u32v(&mut payload, id);
            value(&mut payload, field.value)?;
        }
        u32v(
            &mut payload,
            u32::try_from(row.removed.len()).map_err(io::Error::other)?,
        );
        for key in row.removed {
            u64v(&mut payload, key);
        }
        self.frame(&payload)
    }
    fn complete(&mut self, count: u64) -> io::Result<()> {
        let mut payload = vec![5];
        u64v(&mut payload, count);
        payload.extend(self.digest.clone().finalize());
        self.frame(&payload)?;
        self.output.flush()
    }
}
