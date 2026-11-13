//! Zwischenablage im Editor (Platzhalter, WP10).
//!
//! Vertrag: `copy/cut/paste` wirken auf die aktuelle Auswahl des Editors und sprechen über `platform`
//! mit der System-Zwischenablage (`clipboard_get/_set`). Dieser Stub nutzt nur die interne Zwischenablage des Kerns.

use super::Editor;
use crate::platform::Platform;

pub fn copy(editor: &mut Editor, _platform: &dyn Platform) {
    if !editor.board.sel.is_empty() {
        editor.board.copy_sel();
    }
}

pub fn cut(editor: &mut Editor, _platform: &dyn Platform) {
    if !editor.board.sel.is_empty() {
        editor.board.cut_sel();
    }
}

pub fn paste(editor: &mut Editor, _platform: &dyn Platform) {
    if editor.board.has_clip() {
        editor.board.paste();
    }
}
