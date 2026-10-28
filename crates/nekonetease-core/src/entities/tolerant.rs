//! Deserializers for the wire shapes `NetEase` sends but rarely documents: counts that arrive as
//! JSON floats (`25370443800.0`) or decimal strings, and flags that arrive as `null` for an
//! anonymous viewer. A strict field rejects the whole response, which the UI can only report as
//! "unrecognised data" on every page the row belongs to.
//!
//! Shared rather than duplicated per entity module: the tolerance is one decision, and a second
//! copy is how two modules come to disagree about what a missing count means.

/// Counts arrive as JSON floats often enough — `25370443800.0` for a playlist's plays — that a
/// strict integer field rejects a response the client would otherwise render fine. The fraction
/// is always zero, so the integer part is the count; anything unreadable reads as zero rather
/// than failing the whole response.
///
/// A visitor rather than a `serde_json::Value` round trip: these decode once per song or
/// playlist in a page, and building a DOM node per field is the allocation this avoids.
pub(crate) struct CountVisitor;

impl<'de> serde::de::Visitor<'de> for CountVisitor {
    type Value = u64;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a count")
    }

    fn visit_u64<E>(self, value: u64) -> Result<u64, E> {
        Ok(value)
    }

    fn visit_i64<E>(self, value: i64) -> Result<u64, E> {
        Ok(u64::try_from(value).unwrap_or(0))
    }

    fn visit_f64<E>(self, value: f64) -> Result<u64, E> {
        Ok(float_count(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<u64, E> {
        Ok(value.trim().parse().unwrap_or(0))
    }

    fn visit_unit<E>(self) -> Result<u64, E> {
        Ok(0)
    }

    fn visit_none<E>(self) -> Result<u64, E> {
        Ok(0)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<u64, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

/// Truncating and saturating, which is exactly what a count wants: a negative or NaN entry is
/// nonsense rather than a reason to drop a whole response. Counts sit far below `2^64`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts are below 2^53 on the wire; the clamp keeps every cast defined"
)]
fn float_count(value: f64) -> u64 {
    value.max(0.0) as u64
}

pub(crate) fn deserialize_count<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_any(CountVisitor)
}

/// The same tolerance for a small score, which the wire also sends as a float.
pub(crate) fn deserialize_score<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = deserialize_count(deserializer)?;
    Ok(i32::try_from(value).unwrap_or(0))
}

/// A flag the endpoint may answer with `null` for an anonymous or unknown viewer. Absent is
/// false: the two readings agree on every display decision, and neither may fail a response.
pub(crate) struct FlagVisitor;

impl<'de> serde::de::Visitor<'de> for FlagVisitor {
    type Value = bool;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a flag")
    }

    fn visit_bool<E>(self, value: bool) -> Result<bool, E> {
        Ok(value)
    }

    fn visit_u64<E>(self, value: u64) -> Result<bool, E> {
        Ok(value != 0)
    }

    fn visit_i64<E>(self, value: i64) -> Result<bool, E> {
        Ok(value != 0)
    }

    fn visit_f64<E>(self, value: f64) -> Result<bool, E> {
        Ok(value != 0.0)
    }

    fn visit_str<E>(self, value: &str) -> Result<bool, E> {
        Ok(value == "true" || value == "1")
    }

    fn visit_unit<E>(self) -> Result<bool, E> {
        Ok(false)
    }

    fn visit_none<E>(self) -> Result<bool, E> {
        Ok(false)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<bool, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

pub(crate) fn deserialize_flag<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_any(FlagVisitor)
}
