//! The list box on the host: what the guest keeps in its `CTextListBoxModel` is projected to the host as ONE `ControlKind::List`
//! through the `GuestUi` seam, converting UTF-16 to UTF-8 only here. An S60 list item is the guest's own text in the cell format
//! `"0\tFirstLabel\tSecondLabel\t0"` (aknlists.h:615-621: the first and last cells are icon-array indices); its `\t`-separated cells are
//! the row's cells, unchanged. The sub-cells carry no headings, so the control has none.
//! The projection runs when the list is on screen (`ActivateL`) and whenever the guest tells the list its model changed
//! (`HandleItemAdditionL`, ...); an unchanged list costs no UI call.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{CallbackToken, ControlKind, ControlModel, ControlValue, GuestRect};
use crate::control::{CTRL_I_POSITION, CTRL_I_SIZE};
use crate::listbox::LB_I_MODEL;
use crate::loader::Process;
use crate::objects::{HostObj, ListState};

const TAB: u16 = 9;

/// The cells of one item text: split at tabs.
pub fn split_cells(item: &[u16]) -> Vec<String> {
    item.split(|u| *u == TAB).map(crate::desc::to_utf8).collect()
}

/// The cells of an item of `CAknDoubleGraphicStyleListBox`: `"0\tFirstLabel\tSecondLabel\t0"` (aknlists.h:615-621), the one width every
/// row of the control has. DELETE WHEN a second list style is built (then the width is the style's).
pub const ITEM_CELLS: usize = 4;

/// Rows from item texts, each `width` cells wide: an absent trailing cell is empty; a row with MORE cells than the style has is refused
/// (it would be a different style's item, and dropping a cell would be inventing a row).
pub fn rows_of(items: &[Vec<u16>], width: usize) -> Result<Vec<Vec<String>>, String> {
    items.iter().enumerate().map(|(i, item)| {
        let mut cells = split_cells(item);
        if cells.len() > width { return Err(format!("item {i} has {} cells, the list style has {width}: {:?}", cells.len(), crate::desc::to_utf8(item))); }
        cells.resize(width, String::new());
        Ok(cells)
    }).collect()
}

impl Process {
    pub(crate) fn list_state(&mut self, lb: u32) -> Result<&mut ListState, String> {
        if !matches!(self.obj_get(lb), Some(HostObj::List(_))) {
            self.obj_set(lb, HostObj::List(ListState::default()))?;
        }
        match self.obj_get(lb) { Some(HostObj::List(l)) => Ok(l), _ => Err(format!("{lb:#x}: host state is not a list box's")) }
    }

    /// The rows the list box at `lb` holds now: its model's item texts. `iModel` is the `MListBoxModel` base of the
    /// `CTextListBoxModel` (offset 4), eiklbx.h `MListBoxModel* iModel`.
    pub(crate) fn list_rows(&self, lb: u32) -> Result<Vec<Vec<String>>, String> {
        let iface = self.rd(lb + LB_I_MODEL)?;
        if iface == 0 { return Ok(Vec::new()); }
        let array = self.model_array(iface - 4)?;
        rows_of(&self.mdesc_items(array)?, ITEM_CELLS)
    }

    /// Tell the host what the list box shows now: add its control the first time, then only what changed.
    pub(crate) fn list_sync(&mut self, lb: u32) -> Result<(), String> {
        if !self.list_state(lb)?.activated { return Ok(()); }
        let rows = self.list_rows(lb)?;
        let rd = |p: &Process, o: u32| p.rd(lb + o).map(|v| v as i32);
        let rect = (rd(self, CTRL_I_POSITION)?, rd(self, CTRL_I_POSITION + 4)?, rd(self, CTRL_I_SIZE)?, rd(self, CTRL_I_SIZE + 4)?);
        let win = self.host.window.ok_or_else(|| String::from("no application window yet: the list has nowhere to go"))?;
        let st = self.list_state(lb)?.clone();
        let g = GuestRect { left: rect.0, top: rect.1, width: rect.2, height: rect.3 };
        let selected = if rows.is_empty() { None } else { Some(st.current.min(rows.len() as u32 - 1)) };
        let columns = alloc::vec![String::new(); ITEM_CELLS];
        let ui = self.host.ui.as_mut().ok_or_else(|| String::from("no GuestUi"))?;
        let refused = |e: heirloom_ui_abi::UiRefusal| format!("the UI refused the list: {e}");
        let control = match st.control {
            None => {
                let kind = ControlKind::List { columns, rows: rows.clone(), selected, top: st.top };
                let c = ui.add_control(win, ControlModel { kind, label: String::new(), rect: g, enabled: true, token: CallbackToken(lb as u64) }).map_err(refused)?;
                self.log.push(format!("[list] {lb:#x}: {} row(s) at {}x{}+{}+{}", rows.len(), g.width, g.height, g.left, g.top));
                c
            }
            Some(c) => {
                if st.shown != rows || st.shown_current != st.current || st.shown_top != st.top {
                    ui.set_control(win, c, ControlValue::Rows { rows: rows.clone(), selected, top: st.top }).map_err(refused)?;
                }
                if st.shown_rect != rect { ui.set_control_rect(win, c, g).map_err(refused)?; }
                c
            }
        };
        let l = self.list_state(lb)?;
        (l.control, l.shown, l.shown_current, l.shown_top, l.shown_rect) = (Some(control), rows, l.current, l.top, rect);
        Ok(())
    }
}

#[cfg(feature = "test-hooks")]
impl Process {
    /// TEST HOOK: what `CEikListBox::ActivateL` does to the host list, without the component walk (which runs guest code).
    pub fn test_list_activate(&mut self, lb: u32) -> Result<(), String> {
        self.list_state(lb)?.activated = true;
        self.list_sync(lb)
    }

    /// TEST HOOK: the item texts (UTF-16) the guest's model of the list box at `lb` holds now.
    pub fn test_list_items(&self, lb: u32) -> Result<Vec<Vec<u16>>, String> {
        let iface = self.rd(lb + LB_I_MODEL)?;
        if iface == 0 { return Ok(Vec::new()); }
        self.mdesc_items(self.model_array(iface - 4)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn u(s: &str) -> Vec<u16> { s.encode_utf16().collect() }

    #[test]
    fn an_item_is_split_at_tabs_and_converted_at_the_boundary() {
        assert_eq!(split_cells(&u("0\tFirst\tSecond\t0")), ["0", "First", "Second", "0"]);
        assert_eq!(split_cells(&u("K\u{f6}ln\t\u{1f600}")), ["K\u{f6}ln", "\u{1f600}"]);
        assert_eq!(split_cells(&[]), [""]);
    }

    #[test]
    fn rows_have_the_styles_width_and_absent_cells_are_empty() {
        let rows = rows_of(&[u("a\tb\tc"), u("d"), u("e\tf\tg\th")], 4).unwrap();
        assert_eq!(rows, vec![vec!["a", "b", "c", ""], vec!["d", "", "", ""], vec!["e", "f", "g", "h"]]);
        assert_eq!(rows_of(&[], 4).unwrap(), Vec::<Vec<String>>::new());
    }

    #[test]
    fn an_item_wider_than_the_style_is_refused_not_cut() {
        let e = rows_of(&[u("a\tb"), u("1\t2\t3\t4\t5")], 4).unwrap_err();
        assert!(e.contains("item 1 has 5 cells"), "{e}");
    }
}
