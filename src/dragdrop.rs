//! OLE drag and drop: files dropped on the notch go to the shelf, and shelf
//! files can be dragged out to Explorer or any app.

use std::path::PathBuf;

use windows::core::{implement, Ref, Result, BOOL, HRESULT, HSTRING};
use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HWND, POINTL, S_OK};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl, RegisterDragDrop,
    ReleaseStgMedium, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, DragQueryFileW, ILFree, IShellItemArray, SHCreateShellItemArrayFromIDLists,
    SHParseDisplayName, HDROP,
};

pub enum DragEvent {
    Enter,
    Leave,
    Drop(Vec<PathBuf>),
}

#[implement(IDropTarget)]
struct DropTarget {
    on_event: fn(DragEvent),
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn has_files(obj: &IDataObject) -> bool {
    unsafe { obj.QueryGetData(&hdrop_format()) == S_OK }
}

fn files(obj: &IDataObject) -> Vec<PathBuf> {
    let mut out = Vec::new();
    unsafe {
        let Ok(mut medium) = obj.GetData(&hdrop_format()) else { return out };
        let drop = HDROP(medium.u.hGlobal.0);
        let n = DragQueryFileW(drop, u32::MAX, None);
        for i in 0..n {
            let len = DragQueryFileW(drop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(drop, i, Some(&mut buf));
            out.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
        }
        ReleaseStgMedium(&mut medium);
    }
    out
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, obj: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let ok = obj.as_ref().is_some_and(has_files);
        unsafe { *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        if ok {
            (self.on_event)(DragEvent::Enter);
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            if *effect != DROPEFFECT_NONE {
                *effect = DROPEFFECT_COPY;
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        (self.on_event)(DragEvent::Leave);
        Ok(())
    }

    fn Drop(&self, obj: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let paths = obj.as_ref().map(files).unwrap_or_default();
        unsafe { *effect = if paths.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_COPY } };
        (self.on_event)(DragEvent::Drop(paths));
        Ok(())
    }
}

/// Makes `hwnd` accept dropped files. Needs OleInitialize on this thread.
pub fn register(hwnd: HWND, on_event: fn(DragEvent)) -> Result<()> {
    let target: IDropTarget = DropTarget { on_event }.into();
    unsafe { RegisterDragDrop(hwnd, &target) }
}

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Starts a drag of `paths` (copy). Blocks until the drop ends.
pub fn drag_out(paths: &[PathBuf]) {
    unsafe {
        let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
        for p in paths {
            let mut pidl = std::ptr::null_mut();
            if SHParseDisplayName(&HSTRING::from(p.as_os_str()), None, &mut pidl, 0, None).is_ok() {
                pidls.push(pidl);
            }
        }
        if !pidls.is_empty() {
            let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
            let data = SHCreateShellItemArrayFromIDLists(&consts)
                .and_then(|a: IShellItemArray| a.BindToHandler::<_, IDataObject>(None, &BHID_DataObject));
            if let Ok(data) = data {
                let source: IDropSource = DropSource.into();
                let mut effect = DROPEFFECT::default();
                let _ = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_LINK, &mut effect);
            }
        }
        for p in pidls {
            ILFree(Some(p));
        }
    }
}
