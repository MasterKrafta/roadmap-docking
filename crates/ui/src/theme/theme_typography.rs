/// Typography shared by the application and GPUI adapter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeTypography {
    pub family: &'static str,
    pub title_size: f32,
    pub tab_size: f32,
    pub body_size: f32,
    pub line_height: f32,
}

impl ThemeTypography {
    /// Ensures typography has a usable family and finite positive dimensions.
    pub fn validate(self) -> Result<(), &'static str> {
        if self.family.trim().is_empty() {
            return Err("typography family must not be empty");
        }
        for (name, value) in [
            ("title_size", self.title_size),
            ("tab_size", self.tab_size),
            ("body_size", self.body_size),
            ("line_height", self.line_height),
        ] {
            if !value.is_finite() {
                return Err(match name {
                    "title_size" => "title_size must be finite",
                    "tab_size" => "tab_size must be finite",
                    "body_size" => "body_size must be finite",
                    _ => "line_height must be finite",
                });
            }
            if value <= 0.0 {
                return Err(match name {
                    "title_size" => "title_size must be positive",
                    "tab_size" => "tab_size must be positive",
                    "body_size" => "body_size must be positive",
                    _ => "line_height must be positive",
                });
            }
        }
        Ok(())
    }
}

impl Default for ThemeTypography {
    fn default() -> Self {
        Self {
            family: "Segoe UI",
            title_size: 14.0,
            tab_size: 12.0,
            body_size: 13.0,
            line_height: 18.0,
        }
    }
}
