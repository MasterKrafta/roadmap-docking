/// Typography shared by the application and GPUI adapter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeTypography {
    pub family: &'static str,
    pub title_size: f32,
    pub tab_size: f32,
    pub body_size: f32,
    pub line_height: f32,
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
