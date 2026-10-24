/// Blends an RGB foreground over an RGB background using the foreground weight.
pub(crate) fn blend_rgb(foreground: u32, background: u32, foreground_weight: f32) -> u32 {
    let weight = foreground_weight.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let foreground = ((foreground >> shift) & 0xFF) as f32;
        let background = ((background >> shift) & 0xFF) as f32;
        (foreground * weight + background * (1.0 - weight))
            .round()
            .clamp(0.0, 255.0) as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

#[cfg(test)]
mod tests {
    use super::blend_rgb;

    #[test]
    fn blends_rgb_channels_and_clamps_weight() {
        assert_eq!(blend_rgb(0xFF0000, 0x0000FF, 0.0), 0x0000FF);
        assert_eq!(blend_rgb(0xFF0000, 0x0000FF, 1.0), 0xFF0000);
        assert_eq!(blend_rgb(0xFF0000, 0x0000FF, 0.5), 0x800080);
        assert_eq!(blend_rgb(0xFFFFFF, 0x000000, -1.0), 0x000000);
        assert_eq!(blend_rgb(0xFFFFFF, 0x000000, 2.0), 0xFFFFFF);
    }
}
