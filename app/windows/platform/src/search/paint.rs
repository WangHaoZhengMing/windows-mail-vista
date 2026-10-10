//! Disposable GDI objects and anti-aliased vectors; no DPI-dependent raster assets.
use super::*;

unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, &rect, brush);
        DeleteObject(brush);
    }
}
pub(super) unsafe fn frame(dc: HDC, rect: RECT, state: &State) {
    unsafe {
        fill(dc, rect, WHITE);
        let value = state.dpi.get();
        let thickness = dpi::scale(1, value).max(1);
        let color = if state.focused.get() {
            0x00d48e3a
        } else if state.hot.get() {
            0x00c9a56c
        } else {
            0x00b8afa6
        };
        let pen = CreatePen(PS_SOLID, thickness, color);
        let old_pen = SelectObject(dc, pen);
        let old_brush = SelectObject(dc, GetStockObject(WHITE_BRUSH));
        let inset = thickness / 2;
        RoundRect(
            dc,
            rect.left + inset,
            rect.top + inset,
            rect.right - inset,
            rect.bottom - inset,
            dpi::scale(3, value),
            dpi::scale(3, value),
        );
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(pen);
        if state.focused.get() {
            let pen = CreatePen(PS_SOLID, dpi::scale(1, value), 0x00f6dcc1);
            let old = SelectObject(dc, pen);
            let brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
            let pad = dpi::scale(1, value);
            Rectangle(
                dc,
                pad,
                pad,
                (rect.right - pad).max(pad + 1),
                (rect.bottom - pad).max(pad + 1),
            );
            SelectObject(dc, brush);
            SelectObject(dc, old);
            DeleteObject(pen);
        }
    }
}
struct VectorDrawing {
    graphics: *mut GpGraphics,
    pen: *mut GpPen,
}
impl VectorDrawing {
    unsafe fn new(dc: HDC, width: f32, color: u32) -> Option<Self> {
        unsafe {
            let mut graphics = null_mut();
            let mut pen = null_mut();
            if GdipCreateFromHDC(dc, &mut graphics) != 0 {
                return None;
            }
            if GdipCreatePen1(color, width, UnitPixel, &mut pen) != 0 {
                GdipDeleteGraphics(graphics);
                return None;
            }
            GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias8x8);
            GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapFlat);
            Some(Self { graphics, pen })
        }
    }
}
impl Drop for VectorDrawing {
    fn drop(&mut self) {
        unsafe {
            GdipDeletePen(self.pen);
            GdipDeleteGraphics(self.graphics);
        }
    }
}
pub(super) unsafe fn icon(dc: HDC, rect: RECT, state: &State, draw_state: u32) {
    unsafe {
        let height = (rect.bottom - rect.top).max(1);
        let active = state.hot.get() || draw_state & ODS_SELECTED != 0;
        for y in rect.top..rect.bottom {
            let fraction = (y - rect.top) as f32 / height as f32;
            let shade = if active {
                (
                    246.0 - 35.0 * fraction,
                    252.0 - 23.0 * fraction,
                    255.0 - 8.0 * fraction,
                )
            } else {
                (
                    255.0 - 17.0 * fraction,
                    255.0 - 10.0 * fraction,
                    255.0 - 4.0 * fraction,
                )
            };
            fill(
                dc,
                RECT {
                    left: rect.left,
                    top: y,
                    right: rect.right,
                    bottom: y + 1,
                },
                shade.0 as u32 | ((shade.1 as u32) << 8) | ((shade.2 as u32) << 16),
            );
        }
        let scale = state.dpi.get() as f32 / 96.0;
        let x = (rect.left + rect.right) as f32 / 2.0;
        let y = (rect.top + rect.bottom) as f32 / 2.0;
        let color = if state.has_text.get() {
            0xff657e95
        } else {
            0xff427cae
        };
        let Some(vector) = VectorDrawing::new(dc, 1.75 * scale, color) else {
            return;
        };
        if state.has_text.get() {
            let half = 4.1 * scale;
            GdipDrawLine(
                vector.graphics,
                vector.pen,
                x - half,
                y - half,
                x + half,
                y + half,
            );
            GdipDrawLine(
                vector.graphics,
                vector.pen,
                x - half,
                y + half,
                x + half,
                y - half,
            );
        } else {
            GdipDrawEllipse(
                vector.graphics,
                vector.pen,
                x - 6.0 * scale,
                y - 6.0 * scale,
                9.0 * scale,
                9.0 * scale,
            );
            GdipDrawLine(
                vector.graphics,
                vector.pen,
                x + 1.5 * scale,
                y + 1.5 * scale,
                x + 6.0 * scale,
                y + 6.0 * scale,
            );
            let mut shine = null_mut();
            if GdipCreatePen1(0xffb9ddf0, scale, UnitPixel, &mut shine) == 0 {
                GdipDrawLine(
                    vector.graphics,
                    shine,
                    x - 3.8 * scale,
                    y - 2.8 * scale,
                    x - 2.6 * scale,
                    y - 4.0 * scale,
                );
                GdipDeletePen(shine);
            }
        }
    }
}
