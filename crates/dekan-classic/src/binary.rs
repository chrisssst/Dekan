use std::fmt::Display;

use crate::error::ClassicError;

pub(crate) fn array<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..at.checked_add(N)?)?.try_into().ok()
}

pub(crate) fn u32_le(bytes: &[u8], at: usize) -> Option<u32> {
    array(bytes, at).map(u32::from_le_bytes)
}

pub(crate) struct Reader<'a> {
    pub(crate) bytes: &'a [u8],
    error: fn(String) -> ClassicError,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8], error: fn(String) -> ClassicError) -> Self {
        Self { bytes, error }
    }

    pub(crate) fn fail(&self, what: impl Display) -> ClassicError {
        (self.error)(what.to_string())
    }

    pub(crate) fn take<const N: usize>(&self, at: usize) -> Result<[u8; N], ClassicError> {
        array(self.bytes, at).ok_or_else(|| self.fail(format!("truncated at offset {at}")))
    }

    pub(crate) fn u16(&self, at: usize) -> Result<u16, ClassicError> {
        self.take(at).map(u16::from_le_bytes)
    }

    pub(crate) fn i16(&self, at: usize) -> Result<i16, ClassicError> {
        self.take(at).map(i16::from_le_bytes)
    }

    pub(crate) fn u32(&self, at: usize) -> Result<u32, ClassicError> {
        self.take(at).map(u32::from_le_bytes)
    }

    pub(crate) fn i32(&self, at: usize) -> Result<i32, ClassicError> {
        self.take(at).map(i32::from_le_bytes)
    }

    pub(crate) fn f32(&self, at: usize) -> Result<f32, ClassicError> {
        self.take(at).map(f32::from_le_bytes)
    }

    pub(crate) fn section(&self, at: usize, len: usize) -> Result<&'a [u8], ClassicError> {
        at.checked_add(len)
            .and_then(|end| self.bytes.get(at..end))
            .ok_or_else(|| self.fail(format!("{len} bytes at offset {at} run past the end")))
    }
}
