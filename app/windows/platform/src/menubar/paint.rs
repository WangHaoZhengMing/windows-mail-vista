use super::*;
pub(super) unsafe fn background(window: HWND, dc: HDC) {
    unsafe {
        let mut rect = RECT::default();
        GetClientRect(window, &mut rect);
        SetDCBrushColor(dc, super::super::theme::surface(WHITE));
        FillRect(dc, &rect, GetStockObject(DC_BRUSH));
        let line = CreatePen(PS_SOLID, 1, 0x00e7e4e1);
        let old = SelectObject(dc, line);
        MoveToEx(dc, 0, rect.bottom - 1, null_mut());
        LineTo(dc, rect.right, rect.bottom - 1);
        SelectObject(dc, old);
        DeleteObject(line);
    }
}
pub(super) unsafe fn button(item: &DRAWITEMSTRUCT, state: &State, index: usize) {
    unsafe {
        let selected = state.active.get() == Some(index) || item.itemState & ODS_SELECTED != 0;
        let hot = state.hot.get() == Some(index);
        let color = if selected {
            0x00f7dfca
        } else if hot {
            0x00fff0e3
        } else {
            WHITE
        };
        let brush = CreateSolidBrush(super::super::theme::panel(color));
        FillRect(item.hDC, &item.rcItem, brush);
        DeleteObject(brush);
        if hot || selected {
            let pen = CreatePen(PS_SOLID, 1, if selected { 0x00dba975 } else { 0x00efd5b7 });
            let previous = SelectObject(item.hDC, pen);
            let previous_brush = SelectObject(item.hDC, GetStockObject(NULL_BRUSH));
            RoundRect(
                item.hDC,
                item.rcItem.left,
                item.rcItem.top,
                item.rcItem.right,
                item.rcItem.bottom,
                dpi::scale(3, state.dpi.get()),
                dpi::scale(3, state.dpi.get()),
            );
            SelectObject(item.hDC, previous_brush);
            SelectObject(item.hDC, previous);
            DeleteObject(pen);
        }
        let previous = SelectObject(item.hDC, state.font.get());
        let old_mode = SetBkMode(item.hDC, TRANSPARENT as i32);
        let old_color = SetTextColor(
            item.hDC,
            super::super::theme::text(GetSysColor(COLOR_MENUTEXT)),
        );
        let mut rect = item.rcItem;
        let hide = if state.keyboard.get() {
            0
        } else {
            DT_HIDEPREFIX
        };
        DrawTextW(
            item.hDC,
            state.sections[index].label.as_ptr(),
            -1,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | hide,
        );
        SetTextColor(item.hDC, old_color);
        SetBkMode(item.hDC, old_mode);
        SelectObject(item.hDC, previous);
    }
}
