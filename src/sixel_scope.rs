use crate::data::aircraft::Aircraft;
use crate::data::airports::RunwaySegment;
use crate::data::airspace::AirspaceBoundary;
use crate::raster::{self, Scene};
use crate::theme::Palette;
use crate::trail::TrailStore;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui_image::StatefulImage;
use ratatui_image::picker::Picker;

#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    inner: Rect,
    aircraft: &[Aircraft],
    trails: &TrailStore,
    runways: &[RunwaySegment],
    airspace: &[AirspaceBoundary],
    zoom_radius_nm: f64,
    sweep_angle_deg: f64,
    palette: &Palette,
    picker: &Picker,
    font: &fontdue::Font,
    hide_labels: bool,
) {
    let font_size = picker.font_size();
    let width_px = u32::from(inner.width) * u32::from(font_size.width);
    let height_px = u32::from(inner.height) * u32::from(font_size.height);
    // Match the terminal's own text size instead of an arbitrary constant,
    // per feedback that the labels read smaller than the surrounding UI —
    // see raster::LABEL_FONT_SCALE/LABEL_FONT_MAX_PX for why this is a
    // scale with a cap, not a plain 1:1.
    let label_font_px = raster::label_font_px(f32::from(font_size.height));

    let scene = Scene {
        width_px,
        height_px,
        aircraft,
        trails,
        runways,
        airspace,
        zoom_radius_nm,
        label_font_px,
        sweep_angle_deg,
        palette,
        font,
        hide_labels,
    };
    let image = raster::render(&scene);
    let mut protocol = picker.new_resize_protocol(image::DynamicImage::ImageRgba8(image));
    frame.render_stateful_widget(StatefulImage::new(), inner, &mut protocol);
}
