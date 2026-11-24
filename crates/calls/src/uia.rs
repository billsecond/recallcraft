//! Reading a program's windows the way a screen reader does: Windows' UI Automation, through
//! the `uiautomation` crate's safe API.
//!
//! Each read asks for a window's whole tree at once (a cache request for the name, control type
//! and automation id of every element, in the raw view: the control view leaves out the pages of
//! Chromium apps), so a window costs one round trip
//! to its program rather than one per element. Only the windows of the programs asked about are
//! read, and nothing is clicked or changed. Elsewhere there's nothing to read.
//!
//! Chromium (Chrome, Edge, and the apps built on it, the new Teams and Slack among them) shows
//! its pages to UI Automation only while something listens for its events, as a screen reader
//! does: the reader listens for focus changes, and does nothing with them.

use crate::roster::Element;

/// Reads windows; made once per thread (it sets the thread up for COM).
pub struct Reader {
    #[cfg(target_os = "windows")]
    imp: imp::Reader,
}

impl Reader {
    pub fn new() -> Result<Reader, String> {
        #[cfg(target_os = "windows")]
        {
            Ok(Reader { imp: imp::Reader::new()? })
        }
        #[cfg(not(target_os = "windows"))]
        {
            Err("reading other programs' windows is only done on Windows".to_string())
        }
    }

    /// The titles of the visible windows of the processes `pids`.
    pub fn titles(&self, pids: &[u32]) -> Vec<String> {
        #[cfg(target_os = "windows")]
        {
            imp::windows(pids).into_iter().map(|(_, t)| t).collect()
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = pids;
            Vec::new()
        }
    }

    /// The elements of the visible windows of the processes `pids` (those titled one of
    /// `titles`, when any are), at most `max` of them, each window's from its top.
    pub fn read(&self, pids: &[u32], titles: &[String], max: usize) -> Vec<Element> {
        #[cfg(target_os = "windows")]
        {
            self.imp.read(pids, titles, max)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (pids, titles, max);
            Vec::new()
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use uiautomation::core::UICacheRequest;
    use uiautomation::events::{CustomFocusChangedEventHandlerFn, UIFocusChangedEventHandler};
    use uiautomation::types::{ElementMode, Handle, TreeScope, UIProperty};
    use uiautomation::{UIAutomation, UIElement};
    use winsafe::EnumWindows;

    use super::Element;

    /// How deep a window's tree is followed.
    const MAX_DEPTH: u16 = 60;

    pub struct Reader {
        automation: UIAutomation,
        cache: UICacheRequest,
    }

    fn err(e: uiautomation::Error) -> String {
        format!("UI Automation: {e}")
    }

    impl Reader {
        pub fn new() -> Result<Reader, String> {
            let automation = UIAutomation::new().map_err(err)?;
            let cache = automation.create_cache_request().map_err(err)?;
            for p in [UIProperty::Name, UIProperty::ControlType, UIProperty::AutomationId] {
                cache.add_property(p).map_err(err)?;
            }
            cache.set_tree_scope(TreeScope::Subtree).map_err(err)?;
            // Only what was cached: nothing kept pointing into the other program.
            cache.set_element_mode(ElementMode::None).map_err(err)?;
            // Everything: the control view leaves out the pages of Chromium apps.
            cache.set_tree_filter(automation.create_true_condition().map_err(err)?).map_err(err)?;
            // Listening, as a screen reader does, so Chromium apps show their pages.
            let ignore: Box<CustomFocusChangedEventHandlerFn> = Box::new(|_| Ok(()));
            if let Err(e) = automation.add_focus_changed_event_handler(None, &UIFocusChangedEventHandler::from(ignore)) {
                log::debug!("listening for focus changes: {e}");
            }
            Ok(Reader { automation, cache })
        }

        pub fn read(&self, pids: &[u32], titles: &[String], max: usize) -> Vec<Element> {
            let windows = windows(pids);
            let chosen: Vec<&(isize, String)> = if windows.iter().any(|(_, t)| titles.contains(t)) {
                windows.iter().filter(|(_, t)| titles.contains(t)).collect()
            } else {
                windows.iter().collect()
            };
            let mut out = Vec::new();
            for (hwnd, title) in chosen.into_iter().take(8) {
                if out.len() >= max {
                    break;
                }
                match self.automation.element_from_handle_build_cache(Handle::from(*hwnd), &self.cache) {
                    Ok(root) => walk(&root, &mut out, max),
                    Err(e) => log::debug!("reading a window of {} characters: {e}", title.chars().count()),
                }
            }
            out
        }
    }

    impl Drop for Reader {
        fn drop(&mut self) {
            let _ = self.automation.remove_all_event_handlers();
        }
    }

    /// Every element under `root`, depth first, as the cache has them.
    fn walk(root: &UIElement, out: &mut Vec<Element>, max: usize) {
        let mut stack: Vec<(UIElement, u16)> = vec![(root.clone(), 0)];
        while let Some((e, depth)) = stack.pop() {
            if out.len() >= max {
                return;
            }
            out.push(Element {
                depth,
                kind: e.get_cached_control_type().map(|k| format!("{k:?}")).unwrap_or_default(),
                name: e.get_cached_name().unwrap_or_default(),
                id: e.get_cached_automation_id().unwrap_or_default(),
            });
            if depth >= MAX_DEPTH {
                continue;
            }
            let children = e.get_cached_children().unwrap_or_default();
            // Reversed onto the stack, so they come off in their order.
            for c in children.into_iter().take(2000).rev() {
                stack.push((c, depth + 1));
            }
        }
    }

    /// The visible, titled top-level windows of the processes `pids`: their handles and titles.
    pub fn windows(pids: &[u32]) -> Vec<(isize, String)> {
        let mut out = Vec::new();
        let _ = EnumWindows(|hwnd| {
            if out.len() < 64 && hwnd.IsWindowVisible() {
                let (_, pid) = hwnd.GetWindowThreadProcessId();
                if pids.contains(&pid) {
                    let title = hwnd.GetWindowText().unwrap_or_default();
                    if !title.trim().is_empty() {
                        out.push((hwnd.ptr() as isize, title));
                    }
                }
            }
            true
        });
        out
    }
}
