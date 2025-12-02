use crate::hpack::BytesStr;

#[derive(Debug)]
pub struct Authority(BytesStr);

impl Authority {
    pub fn new(a: BytesStr) -> Self {
        Authority(a)
    }

    pub fn into_inner(self) -> BytesStr {
        self.0
    }

    #[inline]
    pub(crate) fn as_str(&self) -> &str {
        &self.0[..]
    }
}

impl From<&[u8]> for Authority {
    fn from(value: &[u8]) -> Self {
        Authority(BytesStr::unchecked_from_slice(value))
    }
}
