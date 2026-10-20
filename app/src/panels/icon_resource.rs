/// Stable resource identifier for a panel icon.
///
/// Resolution into image data is owned by the UI/platform layer. The panel registry
/// only carries the identifier and therefore has no dependency on GPUI or image APIs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IconResource(String);

impl IconResource {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err("icon resource identifier must not be empty");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
