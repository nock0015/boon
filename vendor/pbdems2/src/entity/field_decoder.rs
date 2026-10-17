use crate::error::Result;
use crate::io::BitReader;
use crate::limits::DecodeLimits;

use super::field_value::FieldValue;
use super::quantized_float::QuantizedFloat;

/// Mutable state shared across field decoders during a single parse pass.
pub struct FieldDecodeContext {
    /// Current tick interval; used by the simulation-time decoder.
    pub tick_interval: f32,
    /// Reusable buffer for string decoding (avoids per-field allocations).
    pub string_buf: Vec<u8>,
    limits: DecodeLimits,
}

impl FieldDecodeContext {
    /// Create a field decoder context with the default resource limits.
    pub fn new(tick_interval: f32) -> Self {
        Self::with_limits(tick_interval, DecodeLimits::default())
    }

    /// Create a field decoder context with explicit resource limits.
    pub fn with_limits(tick_interval: f32, limits: DecodeLimits) -> Self {
        Self {
            tick_interval,
            string_buf: Vec::with_capacity(512),
            limits,
        }
    }

    /// Resource limits enforced by field and entity decoding.
    pub const fn limits(&self) -> DecodeLimits {
        self.limits
    }

    /// Update the tick interval used by simulation-time fields.
    pub fn set_tick_interval(&mut self, tick_interval: f32) {
        self.tick_interval = tick_interval;
    }
}

/// Wire representation used by an unadorned Source 2 `char` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BareCharEncoding {
    /// A null-terminated byte string.
    NullTerminatedString,
    /// An unsigned variable-length integer.
    UnsignedVarint,
}

/// Interpretation of the `qangle_precise` encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreciseQAngleMode {
    /// Recenter each encoded component from `[0, 360)` to `[-180, 180)`.
    Centered,
    /// Preserve each encoded component in its raw `[0, 360)` range.
    Raw,
}

/// Game-supplied policy for Source 2 wire conventions that vary by title.
///
/// The core deliberately does not provide title-named constants. A game
/// adapter should define its own public or private profile and pass it to
/// the serializer parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeProfile {
    bare_char: BareCharEncoding,
    ammo_field: Option<&'static str>,
    pitch_yaw_qangles: bool,
    precise_qangle: PreciseQAngleMode,
    symbolic_array_lengths: &'static [(&'static str, usize)],
    pointer_types: &'static [&'static str],
    dynamic_serializer_types: &'static [&'static str],
}

impl DecodeProfile {
    /// Create a profile with no field-name overrides or two-component angles.
    pub const fn new(bare_char: BareCharEncoding, precise_qangle: PreciseQAngleMode) -> Self {
        Self {
            bare_char,
            ammo_field: None,
            pitch_yaw_qangles: false,
            precise_qangle,
            symbolic_array_lengths: &[],
            pointer_types: &[],
            dynamic_serializer_types: &[],
        }
    }

    /// Decode the named integer field as an `actual_value + 1` ammo value.
    #[must_use]
    pub const fn with_ammo_field(mut self, field_name: &'static str) -> Self {
        self.ammo_field = Some(field_name);
        self
    }

    /// Enable the `qangle_pitch_yaw` two-component angle encoder.
    #[must_use]
    pub const fn with_pitch_yaw_qangles(mut self) -> Self {
        self.pitch_yaw_qangles = true;
        self
    }

    /// Resolve game-defined symbolic fixed-array lengths.
    #[must_use]
    pub const fn with_symbolic_array_lengths(
        mut self,
        lengths: &'static [(&'static str, usize)],
    ) -> Self {
        self.symbolic_array_lengths = lengths;
        self
    }

    /// Treat additional game-defined field types as pointer-presence values.
    #[must_use]
    pub const fn with_pointer_types(mut self, types: &'static [&'static str]) -> Self {
        self.pointer_types = types;
        self
    }

    /// Treat additional game-defined field types as dynamic serializer arrays.
    #[must_use]
    pub const fn with_dynamic_serializer_types(mut self, types: &'static [&'static str]) -> Self {
        self.dynamic_serializer_types = types;
        self
    }
}

/// Describes how to read a single field value from a [`BitReader`](crate::io::BitReader).
///
/// Each variant corresponds to a Source 2 wire encoding. The correct
/// variant is chosen at parse time by [`get_field_metadata`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Decoder {
    /// Single bit.
    Bool,
    /// Zigzag varint, read as signed.
    I64,
    /// Unsigned varint.
    U64,
    /// Fixed-width signed byte, sign-extended to `i64`.
    I64Fixed8,
    /// Fixed-width unsigned byte, extended to `u64`.
    U64Fixed8,
    /// Fixed-width little-endian `u64`.
    U64Fixed64,
    /// Raw 32-bit float, transmitted bit for bit.
    F32NoScale,
    /// Simulation time: a tick count scaled by the tick interval.
    F32SimulationTime,
    /// World coordinate in Source 2's variable-precision `bitcoord` encoding.
    F32Coord,
    /// Unit-normal component: a sign bit plus an 11-bit fraction.
    F32Normal,
    /// Float quantized into a fixed bit width across a bounded range.
    F32Quantized(QuantizedFloat),
    /// A configured ammo field transmitted as `actual_ammo + 1`.
    ///
    /// Decoding subtracts one while guarding against the `0` sentinel.
    Ammo,
    /// Length-prefixed byte string, kept as raw bytes rather than `String`.
    String,
    /// Two components, each read with the inner decoder.
    Vector2(Box<Decoder>),
    /// Three components, each read with the inner decoder.
    Vector3(Box<Decoder>),
    /// Three-component unit normal: two components plus a reconstructed third.
    Vector3Normal,
    /// Four components, each read with the inner decoder.
    Vector4(Box<Decoder>),
    /// Fixed-count float vector for types wider than 4 components (e.g.
    /// `Quaternion` = 4, `CTransform` = 6). Each component uses `inner`.
    FloatVecN {
        /// Number of components to read.
        count: usize,
        /// Decoder applied to each component.
        inner: Box<Decoder>,
    },
    /// Length-prefixed byte blob (`CUtlBinaryBlock`): a varint byte count
    /// followed by that many raw bytes.
    BinaryBlock,
    /// Polymorphic pointer leaf (e.g. `m_pGameModeRules`): a presence bool plus
    /// a ubitvar selecting which concrete sub-serializer is active. We keep the
    /// bool as the field value; the selector affects deeper field paths.
    Poly,
    /// Euler angles at full precision, recentred per the decode profile's
    /// [`PreciseQAngleMode`].
    QAnglePrecise,
    /// Euler angles at full precision with no recentring applied.
    QAnglePreciseRaw,
    /// Pitch and yaw only, each at `bit_count` bits; roll is zero.
    QAnglePitchYaw {
        /// Bits per transmitted component.
        bit_count: usize,
    },
    /// All three angles at `bit_count` bits each.
    QAngleBitCount {
        /// Bits per component.
        bit_count: usize,
    },
    /// Angles carried in the coordinate encoding rather than a bit count.
    QAngleCoord,
    /// Used as a placeholder/invalid decoder.
    Default,
}

impl Decoder {
    /// Read a single field value from the bitstream.
    pub fn decode(&self, ctx: &mut FieldDecodeContext, br: &mut BitReader) -> Result<FieldValue> {
        match self {
            Decoder::Bool => Ok(FieldValue::Bool(br.read_bool()?)),

            Decoder::I64 => Ok(FieldValue::I64(br.read_varint64()?)),

            Decoder::U64 => Ok(FieldValue::U64(br.read_uvarint64()?)),

            Decoder::I64Fixed8 => {
                // Exactly eight bits: reinterpret the byte as two's complement.
                let byte = br.read_bits(8)? as u8;
                Ok(FieldValue::I64(i64::from(i8::from_le_bytes([byte]))))
            }

            Decoder::U64Fixed8 => Ok(FieldValue::U64(br.read_bits(8)?)),

            Decoder::U64Fixed64 => {
                let mut buf = [0u8; 8];
                br.read_bytes(&mut buf)?;
                Ok(FieldValue::U64(u64::from_le_bytes(buf)))
            }

            Decoder::F32NoScale => Ok(FieldValue::F32(br.read_f32()?)),

            Decoder::F32SimulationTime => {
                let ticks = br.read_uvarint32()?;
                Ok(FieldValue::F32(ticks as f32 * ctx.tick_interval))
            }

            Decoder::F32Coord => Ok(FieldValue::F32(br.read_bitcoord()?)),

            Decoder::F32Normal => Ok(FieldValue::F32(br.read_bitnormal()?)),

            Decoder::F32Quantized(qf) => Ok(FieldValue::F32(qf.decode(br)?)),

            Decoder::Ammo => {
                let raw = br.read_uvarint32()?;
                Ok(FieldValue::I32(if raw > 0 { (raw - 1) as i32 } else { 0 }))
            }

            Decoder::String => {
                ctx.string_buf.clear();
                br.read_string_raw_limited(
                    &mut ctx.string_buf,
                    ctx.limits.max_field_string_bytes(),
                )?;
                Ok(FieldValue::String(ctx.string_buf.clone()))
            }

            Decoder::Vector2(inner) => {
                let x = inner.decode_f32(ctx, br)?;
                let y = inner.decode_f32(ctx, br)?;
                Ok(FieldValue::Vector2([x, y]))
            }

            Decoder::Vector3(inner) => {
                let x = inner.decode_f32(ctx, br)?;
                let y = inner.decode_f32(ctx, br)?;
                let z = inner.decode_f32(ctx, br)?;
                Ok(FieldValue::Vector3([x, y, z]))
            }

            Decoder::Vector3Normal => Ok(FieldValue::Vector3(br.read_bitvec3normal()?)),

            Decoder::FloatVecN { count, inner } => {
                let mut v = Vec::with_capacity(*count);
                for _ in 0..*count {
                    v.push(inner.decode_f32(ctx, br)?);
                }
                Ok(FieldValue::FloatVector(v))
            }

            Decoder::BinaryBlock => {
                let n = br.read_uvarint32()? as usize;
                ctx.limits.ensure(
                    "decoded field binary block",
                    n,
                    ctx.limits.max_field_string_bytes(),
                )?;
                ctx.string_buf.clear();
                ctx.string_buf.resize(n, 0);
                br.read_bytes(&mut ctx.string_buf)?;
                Ok(FieldValue::String(ctx.string_buf.clone()))
            }

            Decoder::Poly => {
                let present = br.read_bool()?;
                let _poly_type_index = br.read_ubitvar()?;
                Ok(FieldValue::Bool(present))
            }

            Decoder::Vector4(inner) => {
                let x = inner.decode_f32(ctx, br)?;
                let y = inner.decode_f32(ctx, br)?;
                let z = inner.decode_f32(ctx, br)?;
                let w = inner.decode_f32(ctx, br)?;
                Ok(FieldValue::Vector4([x, y, z, w]))
            }

            Decoder::QAnglePrecise => {
                // Centered precise angles store each present component as a
                // 20-bit angle shifted from [0, 360) to [-180, 180).
                let mut v = [0.0f32; 3];
                let rx = br.read_bool()?;
                let ry = br.read_bool()?;
                let rz = br.read_bool()?;
                if rx {
                    v[0] = br.read_bitangle(20)? - 180.0;
                }
                if ry {
                    v[1] = br.read_bitangle(20)? - 180.0;
                }
                if rz {
                    v[2] = br.read_bitangle(20)? - 180.0;
                }
                Ok(FieldValue::QAngle(v))
            }

            Decoder::QAnglePreciseRaw => {
                let mut value = [0.0; 3];
                let present = [br.read_bool()?, br.read_bool()?, br.read_bool()?];
                for (component, is_present) in value.iter_mut().zip(present) {
                    if is_present {
                        *component = br.read_bitangle(20)?;
                    }
                }
                Ok(FieldValue::QAngle(value))
            }

            Decoder::QAnglePitchYaw { bit_count } => Ok(FieldValue::QAngle([
                br.read_bitangle(*bit_count)?,
                br.read_bitangle(*bit_count)?,
                0.0,
            ])),

            Decoder::QAngleBitCount { bit_count } => {
                let x = br.read_bitangle(*bit_count)?;
                let y = br.read_bitangle(*bit_count)?;
                let z = br.read_bitangle(*bit_count)?;
                Ok(FieldValue::QAngle([x, y, z]))
            }

            Decoder::QAngleCoord => Ok(FieldValue::QAngle(br.read_bitvec3coord()?)),

            Decoder::Default => Ok(FieldValue::U64(br.read_uvarint64()?)),
        }
    }

    /// Helper to decode a field value as f32 (used by vector decoders).
    fn decode_f32(&self, ctx: &mut FieldDecodeContext, br: &mut BitReader) -> Result<f32> {
        match self.decode(ctx, br)? {
            FieldValue::F32(v) => Ok(v),
            _ => Ok(0.0),
        }
    }

    /// Skip a field value without fully decoding it - just advances the bit reader.
    /// This is faster than decode() when we don't need the value.
    #[allow(clippy::only_used_in_recursion)]
    pub fn skip(&self, ctx: &mut FieldDecodeContext, br: &mut BitReader) -> Result<()> {
        match self {
            Decoder::Bool => {
                br.skip_bits(1)?;
            }

            Decoder::I64 => {
                br.skip_varint()?;
            }

            Decoder::U64 => {
                br.skip_varint()?;
            }

            Decoder::I64Fixed8 | Decoder::U64Fixed8 => {
                br.skip_bits(8)?;
            }

            Decoder::U64Fixed64 => {
                br.skip_bits(64)?;
            }

            Decoder::F32NoScale => {
                br.skip_bits(32)?;
            }

            Decoder::F32SimulationTime => {
                br.skip_varint()?;
            }

            Decoder::F32Coord => {
                br.skip_bitcoord()?;
            }

            Decoder::F32Normal => {
                br.skip_bitnormal()?;
            }

            Decoder::F32Quantized(qf) => {
                qf.skip(br)?;
            }

            Decoder::Ammo => {
                br.skip_varint()?;
            }

            Decoder::String => {
                br.skip_string()?;
            }

            Decoder::Vector2(inner) => {
                inner.skip(ctx, br)?;
                inner.skip(ctx, br)?;
            }

            Decoder::Vector3(inner) => {
                inner.skip(ctx, br)?;
                inner.skip(ctx, br)?;
                inner.skip(ctx, br)?;
            }

            Decoder::Vector3Normal => {
                br.skip_bitvec3normal()?;
            }

            Decoder::FloatVecN { count, inner } => {
                for _ in 0..*count {
                    inner.skip(ctx, br)?;
                }
            }

            Decoder::BinaryBlock => {
                let n = br.read_uvarint32()? as usize;
                br.skip_bits(n * 8)?;
            }

            Decoder::Poly => {
                br.skip_bits(1)?;
                br.read_ubitvar()?;
            }

            Decoder::Vector4(inner) => {
                inner.skip(ctx, br)?;
                inner.skip(ctx, br)?;
                inner.skip(ctx, br)?;
                inner.skip(ctx, br)?;
            }

            Decoder::QAnglePrecise | Decoder::QAnglePreciseRaw => {
                let rx = br.read_bool()?;
                let ry = br.read_bool()?;
                let rz = br.read_bool()?;
                if rx {
                    br.skip_bits(20)?;
                }
                if ry {
                    br.skip_bits(20)?;
                }
                if rz {
                    br.skip_bits(20)?;
                }
            }

            Decoder::QAnglePitchYaw { bit_count } => {
                br.skip_bits(*bit_count * 2)?;
            }

            Decoder::QAngleBitCount { bit_count } => {
                br.skip_bits(*bit_count * 3)?;
            }

            Decoder::QAngleCoord => {
                br.skip_bitvec3coord()?;
            }

            Decoder::Default => {
                br.skip_varint()?;
            }
        }
        Ok(())
    }
}

/// Special descriptor for fields that need non-standard handling
/// (arrays, pointers, or nested serializers).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FieldSpecialDescriptor {
    /// Fixed-length array (e.g. `int32[4]`).
    FixedArray {
        /// Element count declared by the schema.
        length: usize,
    },
    /// Variable-length array of a primitive type (e.g. `CNetworkUtlVectorBase<int32>`).
    DynamicArray {
        /// Decoder applied to each element.
        inner_decoder: Decoder,
    },
    /// Variable-length array whose elements have a nested serializer.
    DynamicSerializerArray,
    /// Pointer / entity handle (encoded as a single boolean "present" flag).
    Pointer,
}

/// Metadata about how to decode a field.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FieldMetadata {
    /// Decoder selected for the field's wire encoding.
    pub decoder: Decoder,
    /// Array or pointer shape layered on top of `decoder`, if any.
    pub special: Option<FieldSpecialDescriptor>,
}

impl Default for FieldMetadata {
    fn default() -> Self {
        Self {
            decoder: Decoder::Default,
            special: None,
        }
    }
}

impl FieldMetadata {
    /// Returns `true` for variable-length arrays of either element kind.
    pub fn is_dynamic_array(&self) -> bool {
        matches!(
            self.special,
            Some(FieldSpecialDescriptor::DynamicArray { .. })
                | Some(FieldSpecialDescriptor::DynamicSerializerArray)
        )
    }

    /// Returns `true` for arrays with a length fixed by the schema.
    pub fn is_fixed_array(&self) -> bool {
        matches!(
            self.special,
            Some(FieldSpecialDescriptor::FixedArray { .. })
        )
    }

    /// Element count for a fixed-length array, or `None` for other shapes.
    pub fn fixed_array_length(&self) -> Option<usize> {
        match &self.special {
            Some(FieldSpecialDescriptor::FixedArray { length }) => Some(*length),
            _ => None,
        }
    }

    /// Returns `true` for variable-length arrays whose elements have their own
    /// nested serializer rather than a primitive decoder.
    pub fn is_dynamic_serializer_array(&self) -> bool {
        matches!(
            self.special,
            Some(FieldSpecialDescriptor::DynamicSerializerArray)
        )
    }

    /// Returns `true` for pointer fields, which encode only a presence bit.
    pub fn is_pointer(&self) -> bool {
        matches!(self.special, Some(FieldSpecialDescriptor::Pointer))
    }

    /// Metadata for one element of a dynamic array.
    ///
    /// Returns the default metadata when this field is not a dynamic array.
    pub fn dynamic_array_inner_metadata(&self) -> FieldMetadata {
        match &self.special {
            Some(FieldSpecialDescriptor::DynamicArray { inner_decoder }) => FieldMetadata {
                decoder: inner_decoder.clone(),
                special: None,
            },
            _ => FieldMetadata::default(),
        }
    }
}

/// Build a float decoder based on field properties.
fn build_f32_decoder(
    var_name: &str,
    bit_count: Option<i32>,
    low_value: Option<f32>,
    high_value: Option<f32>,
    encode_flags: Option<i32>,
    var_encoder: Option<&str>,
) -> Decoder {
    // Simulation time special case
    if var_name == "m_flSimulationTime" || var_name == "m_flAnimTime" {
        return Decoder::F32SimulationTime;
    }

    // Check var_encoder
    if let Some(encoder) = var_encoder {
        match encoder {
            "coord" => return Decoder::F32Coord,
            "normal" => return Decoder::F32Normal,
            _ => {}
        }
    }

    let bc = bit_count.unwrap_or(0);
    if bc == 0 || bc == 32 {
        return Decoder::F32NoScale;
    }

    // Quantized float
    match QuantizedFloat::new(
        bc,
        encode_flags.unwrap_or(0),
        low_value.unwrap_or(0.0),
        high_value.unwrap_or(0.0),
    ) {
        Ok(qf) => Decoder::F32Quantized(qf),
        Err(_) => Decoder::F32NoScale,
    }
}

/// Determine the [`FieldMetadata`] (decoder + special descriptor) for a serializer field.
///
/// This is the main dispatch function that maps Source 2 network field descriptions
/// to the correct binary decoder. It inspects the type string, field name, and
/// encoder hints to choose the appropriate [`Decoder`] variant and, when the field
/// represents an array or pointer, attaches a [`FieldSpecialDescriptor`].
///
/// # Parameters
///
/// * profile — game-supplied overrides for varying wire conventions.
/// * `var_type` — the Source 2 type name (e.g. `"int32"`, `"Vector"`, `"CBaseEntity*"`,
///   `"CNetworkUtlVectorBase< float32 >"`). Pointer suffix (`*`), array brackets
///   (`[N]`), and generic angle brackets (`< T >`) are all handled.
/// * `var_name` — the field name (e.g. `"m_flSimulationTime"`). Certain names trigger
///   special-case decoders.
/// * `bit_count` — optional bit width from the serializer; used for quantized floats
///   and `QAngle` variants.
/// * `low_value` / `high_value` — optional range bounds for quantized float encoding.
/// * `encode_flags` — optional flags passed to [`QuantizedFloat`] when constructing a
///   quantized decoder.
/// * `var_encoder` — optional encoder hint string (e.g. `"coord"`, `"normal"`,
///   `"qangle_precise"`, `"fixed64"`).
/// * `has_field_serializer` — `true` when the field carries a nested serializer,
///   which upgrades dynamic arrays to [`FieldSpecialDescriptor::DynamicSerializerArray`].
#[allow(clippy::too_many_arguments)]
pub fn get_field_metadata(
    profile: DecodeProfile,
    var_type: &str,
    var_name: &str,
    bit_count: Option<i32>,
    low_value: Option<f32>,
    high_value: Option<f32>,
    encode_flags: Option<i32>,
    var_encoder: Option<&str>,
    has_field_serializer: bool,
) -> FieldMetadata {
    // Some games transmit a configured ammo field as `ammo + 1`
    // regardless of the field's declared integer type.
    if profile.ammo_field == Some(var_name) {
        return FieldMetadata {
            decoder: Decoder::Ammo,
            special: None,
        };
    }

    // Parse the type to determine category
    let trimmed = var_type.trim();

    // Pointer types
    if trimmed.ends_with('*') {
        return FieldMetadata {
            decoder: Decoder::Bool,
            special: Some(FieldSpecialDescriptor::Pointer),
        };
    }

    // Array types: type[length]
    if let Some(bracket_pos) = trimmed.find('[')
        && trimmed.ends_with(']')
    {
        let base = trimmed[..bracket_pos].trim();
        let len_str = trimmed[bracket_pos + 1..trimmed.len() - 1].trim();

        // char[N] is a string
        if base == "char" {
            return FieldMetadata {
                decoder: Decoder::String,
                special: None,
            };
        }

        let length = match len_str.parse::<usize>() {
            Ok(length) => length,
            Err(_) => profile
                .symbolic_array_lengths
                .iter()
                .find_map(|(symbol, length)| (*symbol == len_str).then_some(*length))
                .unwrap_or(64),
        };

        let inner = get_field_metadata(
            profile,
            base,
            var_name,
            bit_count,
            low_value,
            high_value,
            encode_flags,
            var_encoder,
            has_field_serializer,
        );

        return FieldMetadata {
            decoder: inner.decoder,
            special: Some(FieldSpecialDescriptor::FixedArray { length }),
        };
    }

    // Generic/template types: CNetworkUtlVectorBase< T >
    if let Some(angle_pos) = trimmed.find('<')
        && let Some(close_pos) = trimmed.rfind('>')
    {
        let base = trimmed[..angle_pos].trim();
        let inner_type = trimmed[angle_pos + 1..close_pos].trim();

        let is_vector_base = matches!(
            base,
            "CNetworkUtlVectorBase" | "CUtlVectorEmbeddedNetworkVar" | "CUtlVector"
        );

        if is_vector_base {
            if has_field_serializer {
                return FieldMetadata {
                    decoder: Decoder::U64,
                    special: Some(FieldSpecialDescriptor::DynamicSerializerArray),
                };
            }

            let inner = get_field_metadata(
                profile,
                inner_type,
                var_name,
                bit_count,
                low_value,
                high_value,
                encode_flags,
                var_encoder,
                has_field_serializer,
            );

            return FieldMetadata {
                decoder: Decoder::U64,
                special: Some(FieldSpecialDescriptor::DynamicArray {
                    inner_decoder: inner.decoder,
                }),
            };
        }

        // For non-vector templates, decode as the base type
        return get_field_metadata(
            profile,
            base,
            var_name,
            bit_count,
            low_value,
            high_value,
            encode_flags,
            var_encoder,
            has_field_serializer,
        );
    }

    if profile.pointer_types.contains(&trimmed) {
        return FieldMetadata {
            decoder: Decoder::Bool,
            special: Some(FieldSpecialDescriptor::Pointer),
        };
    }

    // Apply the wire encoder to scalar values after resolving array elements.
    if var_encoder == Some("fixed8") {
        return FieldMetadata {
            decoder: match trimmed {
                "int8" | "int16" | "int32" | "int64" => Decoder::I64Fixed8,
                _ => Decoder::U64Fixed8,
            },
            special: None,
        };
    }

    // Identify the base type
    match trimmed {
        // Primitives
        "int8" | "int16" | "int32" | "int64" => FieldMetadata {
            decoder: Decoder::I64,
            special: None,
        },

        "bool" => FieldMetadata {
            decoder: Decoder::Bool,
            special: None,
        },

        "float32" | "CNetworkedQuantizedFloat" | "GameTime_t" => {
            let decoder = build_f32_decoder(
                var_name,
                bit_count,
                low_value,
                high_value,
                encode_flags,
                var_encoder,
            );
            FieldMetadata {
                decoder,
                special: None,
            }
        }

        // Pointer types: entity body/component handles transmitted as a
        // single boolean "present" flag on the wire.
        "CBodyComponentBaseAnimating"
        | "CBodyComponentBaseAnimatingOverlay"
        | "CBodyComponentBaseModelEntity"
        | "CBodyComponent"
        | "CBodyComponentSkeletonInstance"
        | "CBodyComponentPoint"
        | "CLightComponent"
        | "CRenderComponent"
        | "C_BodyComponentBaseAnimating"
        | "C_BodyComponentBaseAnimatingOverlay"
        | "CPhysicsComponent" => FieldMetadata {
            decoder: Decoder::Bool,
            special: Some(FieldSpecialDescriptor::Pointer),
        },

        // String types
        "CUtlSymbolLarge" | "CUtlString" | "CGlobalSymbol" => FieldMetadata {
            decoder: Decoder::String,
            special: None,
        },

        "char" => FieldMetadata {
            decoder: match profile.bare_char {
                BareCharEncoding::NullTerminatedString => Decoder::String,
                BareCharEncoding::UnsignedVarint => Decoder::U64,
            },
            special: None,
        },

        // Length-prefixed byte blob.
        "CUtlBinaryBlock" => FieldMetadata {
            decoder: Decoder::BinaryBlock,
            special: None,
        },

        // Wide float vectors (more than 4 components).
        "Quaternion" | "CTransform" => {
            let count = if trimmed == "CTransform" { 6 } else { 4 };
            let inner = build_f32_decoder(
                var_name,
                bit_count,
                low_value,
                high_value,
                encode_flags,
                var_encoder,
            );
            FieldMetadata {
                decoder: Decoder::FloatVecN {
                    count,
                    inner: Box::new(inner),
                },
                special: None,
            }
        }

        // Angle conventions are selected by the game adapter's profile.
        "QAngle" => {
            let bit_count = bit_count.unwrap_or(0) as usize;
            let decoder = match var_encoder {
                Some("qangle_pitch_yaw") if profile.pitch_yaw_qangles => {
                    Decoder::QAnglePitchYaw { bit_count }
                }
                Some("qangle_precise") => match profile.precise_qangle {
                    PreciseQAngleMode::Centered => Decoder::QAnglePrecise,
                    PreciseQAngleMode::Raw => Decoder::QAnglePreciseRaw,
                },
                _ if bit_count != 0 => Decoder::QAngleBitCount { bit_count },
                _ => Decoder::QAngleCoord,
            };
            FieldMetadata {
                decoder,
                special: None,
            }
        }

        // Vector types
        "Vector" | "VectorWS" => {
            if var_encoder == Some("normal") {
                FieldMetadata {
                    decoder: Decoder::Vector3Normal,
                    special: None,
                }
            } else {
                let inner = build_f32_decoder(
                    var_name,
                    bit_count,
                    low_value,
                    high_value,
                    encode_flags,
                    var_encoder,
                );
                FieldMetadata {
                    decoder: Decoder::Vector3(Box::new(inner)),
                    special: None,
                }
            }
        }

        "Vector2D" => {
            let inner = build_f32_decoder(
                var_name,
                bit_count,
                low_value,
                high_value,
                encode_flags,
                var_encoder,
            );
            FieldMetadata {
                decoder: Decoder::Vector2(Box::new(inner)),
                special: None,
            }
        }

        "Vector4D" => {
            let inner = build_f32_decoder(
                var_name,
                bit_count,
                low_value,
                high_value,
                encode_flags,
                var_encoder,
            );
            FieldMetadata {
                decoder: Decoder::Vector4(Box::new(inner)),
                special: None,
            }
        }

        // Game-defined dynamic serializer arrays.
        _ if profile.dynamic_serializer_types.contains(&trimmed) => FieldMetadata {
            decoder: Decoder::U64,
            special: Some(FieldSpecialDescriptor::DynamicSerializerArray),
        },

        // Default: unsigned integer
        _ => {
            let decoder = if var_encoder == Some("fixed64") {
                Decoder::U64Fixed64
            } else {
                Decoder::U64
            };
            FieldMetadata {
                decoder,
                special: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::BitReader;

    const TEST_PROFILE: DecodeProfile = DecodeProfile::new(
        BareCharEncoding::NullTerminatedString,
        PreciseQAngleMode::Centered,
    )
    .with_ammo_field("m_ammo");

    fn meta(var_type: &str, var_name: &str) -> FieldMetadata {
        get_field_metadata(
            TEST_PROFILE,
            var_type,
            var_name,
            None,
            None,
            None,
            None,
            None,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn meta_full(
        var_type: &str,
        var_name: &str,
        bit_count: Option<i32>,
        low: Option<f32>,
        high: Option<f32>,
        encode_flags: Option<i32>,
        var_encoder: Option<&str>,
        has_fs: bool,
    ) -> FieldMetadata {
        get_field_metadata(
            TEST_PROFILE,
            var_type,
            var_name,
            bit_count,
            low,
            high,
            encode_flags,
            var_encoder,
            has_fs,
        )
    }

    // ── get_field_metadata dispatch ──

    #[test]
    fn pointer_type() {
        let m = meta("CBaseEntity*", "m_hOwner");
        assert!(matches!(m.decoder, Decoder::Bool));
        assert!(m.is_pointer());
    }

    #[test]
    fn bool_type() {
        let m = meta("bool", "m_bActive");
        assert!(matches!(m.decoder, Decoder::Bool));
        assert!(m.special.is_none());
    }

    #[test]
    fn int32_type() {
        let m = meta("int32", "m_iHealth");
        assert!(matches!(m.decoder, Decoder::I64));
    }

    #[test]
    fn float32_no_scale() {
        let m = meta("float32", "m_flValue");
        assert!(matches!(m.decoder, Decoder::F32NoScale));
    }

    #[test]
    fn simulation_time() {
        let m = meta("float32", "m_flSimulationTime");
        assert!(matches!(m.decoder, Decoder::F32SimulationTime));
    }

    #[test]
    fn coord_encoder() {
        let m = meta_full(
            "float32",
            "m_x",
            None,
            None,
            None,
            None,
            Some("coord"),
            false,
        );
        assert!(matches!(m.decoder, Decoder::F32Coord));
    }

    #[test]
    fn quantized_float() {
        let m = meta_full(
            "float32",
            "m_val",
            Some(8),
            Some(0.0),
            Some(255.0),
            None,
            None,
            false,
        );
        assert!(matches!(m.decoder, Decoder::F32Quantized(_)));
    }

    #[test]
    fn string_utl_symbol() {
        let m = meta("CUtlSymbolLarge", "m_iszName");
        assert!(matches!(m.decoder, Decoder::String));
    }

    #[test]
    fn char_array_is_string() {
        let m = meta("char[256]", "m_szName");
        assert!(matches!(m.decoder, Decoder::String));
        assert!(!m.is_fixed_array());
    }

    #[test]
    fn int32_array_is_fixed_array() {
        let m = meta("int32[4]", "m_values");
        assert!(m.is_fixed_array());
        assert_eq!(m.fixed_array_length(), Some(4));
    }

    #[test]
    fn dynamic_array_without_serializer() {
        let m = meta("CNetworkUtlVectorBase< int32 >", "m_items");
        assert!(m.is_dynamic_array());
        assert!(!m.is_dynamic_serializer_array());
    }

    #[test]
    fn dynamic_serializer_array() {
        let m = meta_full(
            "CNetworkUtlVectorBase< SomeType >",
            "m_items",
            None,
            None,
            None,
            None,
            None,
            true,
        );
        assert!(m.is_dynamic_serializer_array());
    }

    #[test]
    fn qangle_no_encoder_no_bits() {
        let m = meta("QAngle", "m_angle");
        assert!(matches!(m.decoder, Decoder::QAngleCoord));
    }

    #[test]
    fn qangle_with_bitcount() {
        let m = meta_full("QAngle", "m_angle", Some(16), None, None, None, None, false);
        assert!(matches!(
            m.decoder,
            Decoder::QAngleBitCount { bit_count: 16 }
        ));
    }

    #[test]
    fn qangle_precise() {
        let m = meta_full(
            "QAngle",
            "m_angle",
            Some(10),
            None,
            None,
            None,
            Some("qangle_precise"),
            false,
        );
        assert!(matches!(m.decoder, Decoder::QAnglePrecise));
    }

    #[test]
    fn configured_ammo_field_uses_ammo_decoder() {
        // `m_ammo` is name-special-cased regardless of its declared type.
        let m = meta("uint16", "m_ammo");
        assert!(matches!(m.decoder, Decoder::Ammo));
        assert!(m.special.is_none());
    }

    #[test]
    fn binary_block_type() {
        let m = meta("CUtlBinaryBlock", "m_topology");
        assert!(matches!(m.decoder, Decoder::BinaryBlock));
    }

    #[test]
    fn quaternion_and_transform_are_float_vectors() {
        assert!(matches!(
            meta("Quaternion", "m_q").decoder,
            Decoder::FloatVecN { count: 4, .. }
        ));
        assert!(matches!(
            meta("CTransform", "m_t").decoder,
            Decoder::FloatVecN { count: 6, .. }
        ));
    }

    #[test]
    fn bare_char_is_string() {
        assert!(matches!(meta("char", "m_c").decoder, Decoder::String));
    }

    #[test]
    fn decode_ammo_subtracts_one() {
        // Ammo transmits `actual + 1` as a varint; value 6 → 5.
        let data = [0x06];
        let mut br = BitReader::new(&data);
        let mut ctx = FieldDecodeContext::new(1.0 / 64.0);
        let val = Decoder::Ammo.decode(&mut ctx, &mut br).unwrap();
        assert!(matches!(val, FieldValue::I32(5)));
    }

    #[test]
    fn decode_ammo_zero_stays_zero() {
        let data = [0x00];
        let mut br = BitReader::new(&data);
        let mut ctx = FieldDecodeContext::new(1.0 / 64.0);
        let val = Decoder::Ammo.decode(&mut ctx, &mut br).unwrap();
        assert!(matches!(val, FieldValue::I32(0)));
    }

    #[test]
    fn decode_binary_block_reads_len_then_bytes() {
        // varint length 3, then 3 bytes "abc", then a trailing byte.
        let data = [0x03, b'a', b'b', b'c', 0xFF];
        let mut br = BitReader::new(&data);
        let mut ctx = FieldDecodeContext::new(1.0 / 64.0);
        let val = Decoder::BinaryBlock.decode(&mut ctx, &mut br).unwrap();
        match val {
            FieldValue::String(bytes) => assert_eq!(&bytes, b"abc"),
            other => panic!("expected String, got {other:?}"),
        }
        // Cursor should sit exactly on the trailing byte (32 bits consumed).
        assert_eq!(br.position(), 32);
    }

    #[test]
    fn decode_poly_reads_bool_and_ubitvar() {
        // presence bit 1, then a 6-bit ubitvar (top two bits 00 → 6-bit value).
        let data = [0b0000_0011];
        let mut br = BitReader::new(&data);
        let mut ctx = FieldDecodeContext::new(1.0 / 64.0);
        let val = Decoder::Poly.decode(&mut ctx, &mut br).unwrap();
        assert!(matches!(val, FieldValue::Bool(true)));
        // 1 bit (bool) + 6 bits (ubitvar) consumed.
        assert_eq!(br.position(), 7);
    }

    // ── Decoder::decode with BitReader ──

    #[test]
    fn decode_bool_from_1bit() {
        let data = [0x01];
        let mut br = BitReader::new(&data);
        let mut ctx = FieldDecodeContext::new(1.0 / 64.0);
        let val = Decoder::Bool.decode(&mut ctx, &mut br).unwrap();
        assert!(matches!(val, FieldValue::Bool(true)));
    }

    #[test]
    fn decode_f32_no_scale() {
        let bytes = 1.5f32.to_le_bytes();
        let mut br = BitReader::new(&bytes);
        let mut ctx = FieldDecodeContext::new(1.0 / 64.0);
        let val = Decoder::F32NoScale.decode(&mut ctx, &mut br).unwrap();
        if let FieldValue::F32(f) = val {
            assert!((f - 1.5).abs() < f32::EPSILON);
        } else {
            panic!("expected F32");
        }
    }

    #[test]
    fn decode_string_null_terminated() {
        let data = b"hello\0";
        let mut br = BitReader::new(data);
        let mut ctx = FieldDecodeContext::new(1.0 / 64.0);
        let val = Decoder::String.decode(&mut ctx, &mut br).unwrap();
        if let FieldValue::String(s) = val {
            assert_eq!(&s, b"hello");
        } else {
            panic!("expected String");
        }
    }

    #[test]
    fn fixed8_decode_and_skip_preserve_alignment() {
        use crate::test_utils::BitWriter;

        let mut ctx = FieldDecodeContext::new(1.0 / 64.0);
        for offset in 0..8 {
            for byte in 0..=u8::MAX {
                let mut writer = BitWriter::default();
                writer.push_bits(0, offset);
                writer.push_bytes(&[byte, 0xa5]);
                let bytes = writer.finish();
                for (decoder, expected) in [
                    (Decoder::U64Fixed8, FieldValue::U64(u64::from(byte))),
                    (
                        Decoder::I64Fixed8,
                        FieldValue::I64(i64::from(i8::from_le_bytes([byte]))),
                    ),
                ] {
                    let mut reader = BitReader::new(&bytes);
                    reader.skip_bits(offset).unwrap();
                    let mut skipped = BitReader::new(&bytes);
                    skipped.skip_bits(offset).unwrap();
                    match (decoder.decode(&mut ctx, &mut reader).unwrap(), expected) {
                        (FieldValue::I64(actual), FieldValue::I64(expected)) => {
                            assert_eq!(actual, expected)
                        }
                        (FieldValue::U64(actual), FieldValue::U64(expected)) => {
                            assert_eq!(actual, expected)
                        }
                        values => panic!("unexpected fixed8 value types: {values:?}"),
                    }
                    decoder.skip(&mut ctx, &mut skipped).unwrap();
                    assert_eq!(reader.position(), offset + 8);
                    assert_eq!(skipped.position(), reader.position());
                    assert_eq!(reader.read_bits(8).unwrap(), 0xa5);
                    assert_eq!(skipped.read_bits(8).unwrap(), 0xa5);
                }
            }
        }
    }

    #[test]
    fn fixed8_rejects_truncated_bytes() {
        let mut ctx = FieldDecodeContext::new(1.0 / 64.0);
        for offset in 1..=8 {
            for decoder in [Decoder::I64Fixed8, Decoder::U64Fixed8] {
                let mut reader = BitReader::new(&[0xff]);
                reader.skip_bits(offset).unwrap();
                let mut skipped = BitReader::new(&[0xff]);
                skipped.skip_bits(offset).unwrap();
                assert!(decoder.decode(&mut ctx, &mut reader).is_err());
                assert!(decoder.skip(&mut ctx, &mut skipped).is_err());
            }
        }
    }

    #[test]
    fn fixed8_arrays_keep_their_length_encoding() {
        let metadata = |var_type| {
            meta_full(
                var_type,
                "m_values",
                None,
                None,
                None,
                None,
                Some("fixed8"),
                false,
            )
        };
        let fixed = metadata("uint8[4]");
        assert_eq!(fixed.fixed_array_length(), Some(4));
        assert!(matches!(fixed.decoder, Decoder::U64Fixed8));

        let dynamic = metadata("CNetworkUtlVectorBase< int8 >");
        assert!(matches!(dynamic.decoder, Decoder::U64));
        assert!(matches!(
            dynamic.special,
            Some(FieldSpecialDescriptor::DynamicArray {
                inner_decoder: Decoder::I64Fixed8
            })
        ));
    }

    // ── FieldMetadata helpers ──

    #[test]
    fn field_metadata_helpers() {
        let dyn_arr = FieldMetadata {
            decoder: Decoder::U64,
            special: Some(FieldSpecialDescriptor::DynamicArray {
                inner_decoder: Decoder::I64,
            }),
        };
        assert!(dyn_arr.is_dynamic_array());
        assert!(!dyn_arr.is_fixed_array());
        assert!(!dyn_arr.is_pointer());

        let fixed = FieldMetadata {
            decoder: Decoder::I64,
            special: Some(FieldSpecialDescriptor::FixedArray { length: 8 }),
        };
        assert!(fixed.is_fixed_array());
        assert_eq!(fixed.fixed_array_length(), Some(8));

        let ptr = FieldMetadata {
            decoder: Decoder::Bool,
            special: Some(FieldSpecialDescriptor::Pointer),
        };
        assert!(ptr.is_pointer());
    }
}
