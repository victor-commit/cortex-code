//! Light-canvas preview of the shared five-theme gallery.

use ratatui::Frame;

use crate::chrome::{self, ThemeGalleryScheme};

pub fn render(f: &mut Frame) {
    chrome::theme_gallery(f, ThemeGalleryScheme::Light);
}
