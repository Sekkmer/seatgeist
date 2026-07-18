use libseatgeist::SeatgeistError;

use super::Result;

// Keep this in protocol order so role ids from GetRole and Cache.GetItems can be
// normalized without relying on the optional, toolkit-specific GetRoleName.
const ATSPI_ROLE_NAMES: [&str; 130] = [
    "invalid",
    "accelerator label",
    "alert",
    "animation",
    "arrow",
    "calendar",
    "canvas",
    "check box",
    "check menu item",
    "color chooser",
    "column header",
    "combo box",
    "date editor",
    "desktop icon",
    "desktop frame",
    "dial",
    "dialog",
    "directory pane",
    "drawing area",
    "file chooser",
    "filler",
    "focus traversable",
    "font chooser",
    "frame",
    "glass pane",
    "html container",
    "icon",
    "image",
    "internal frame",
    "label",
    "layered pane",
    "list",
    "list item",
    "menu",
    "menu bar",
    "menu item",
    "option pane",
    "page tab",
    "page tab list",
    "panel",
    "password text",
    "popup menu",
    "progress bar",
    "button",
    "radio button",
    "radio menu item",
    "root pane",
    "row header",
    "scroll bar",
    "scroll pane",
    "separator",
    "slider",
    "spin button",
    "split pane",
    "status bar",
    "table",
    "table cell",
    "table column header",
    "table row header",
    "tearoff menu item",
    "terminal",
    "text",
    "toggle button",
    "tool bar",
    "tool tip",
    "tree",
    "tree table",
    "unknown",
    "viewport",
    "window",
    "extended",
    "header",
    "footer",
    "paragraph",
    "ruler",
    "application",
    "autocomplete",
    "editbar",
    "embedded",
    "entry",
    "chart",
    "caption",
    "document frame",
    "heading",
    "page",
    "section",
    "redundant object",
    "form",
    "link",
    "input method window",
    "table row",
    "tree item",
    "document spreadsheet",
    "document presentation",
    "document text",
    "document web",
    "document email",
    "comment",
    "list box",
    "grouping",
    "image map",
    "notification",
    "info bar",
    "level bar",
    "title bar",
    "block quote",
    "audio",
    "video",
    "definition",
    "article",
    "landmark",
    "log",
    "marquee",
    "math",
    "rating",
    "timer",
    "static",
    "math fraction",
    "math root",
    "subscript",
    "superscript",
    "description list",
    "description term",
    "description value",
    "footnote",
    "content deletion",
    "content insertion",
    "mark",
    "suggestion",
    "push button menu",
];

pub(super) fn resolve_role_id<F>(role_output: &str, localized: F) -> Result<String>
where
    F: FnOnce() -> Result<String>,
{
    let role_id = parse_u32_value(role_output)?;
    resolve_role_value(role_id, localized)
}

pub(super) fn resolve_role_value<F>(role_id: u32, localized: F) -> Result<String>
where
    F: FnOnce() -> Result<String>,
{
    let standard = ATSPI_ROLE_NAMES.get(role_id as usize).copied();
    if matches!(standard, Some("unknown" | "extended") | None) {
        let localized = localized()?;
        if !localized.trim().is_empty() {
            return Ok(localized);
        }
    }
    Ok(standard.unwrap_or("unknown").to_string())
}

fn parse_u32_value(output: &str) -> Result<u32> {
    output
        .split_whitespace()
        .rev()
        .find_map(|value| value.parse::<u32>().ok())
        .ok_or_else(|| SeatgeistError::InvalidRequest(format!("expected u32 value: {output}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_numeric_roles_exported_by_accesskit() {
        for (output, expected) in [
            ("u 23\n", "frame"),
            ("u 29\n", "label"),
            ("u 39\n", "panel"),
            ("u 40\n", "password text"),
            ("u 43\n", "button"),
            ("u 79\n", "entry"),
            ("u 116\n", "static"),
        ] {
            let role = resolve_role_id(output, || {
                Err(SeatgeistError::InvalidRequest(
                    "known numeric roles must not require localized fallback".to_string(),
                ))
            })
            .expect("numeric role resolves");
            assert_eq!(role, expected);
        }
    }

    #[test]
    fn uses_localized_role_for_extended_or_future_roles() {
        let extended = resolve_role_id("u 70\n", || Ok("custom widget".to_string()))
            .expect("extended role resolves");
        assert_eq!(extended, "custom widget");

        let future = resolve_role_id("u 999\n", || Ok("future widget".to_string()))
            .expect("future role resolves");
        assert_eq!(future, "future widget");
    }
}
