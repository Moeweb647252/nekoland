use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    pub cursor_theme: String,
    pub border_color: String,
    pub background_color: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: "catppuccin-latte".to_owned(),
            cursor_theme: "default".to_owned(),
            border_color: "#5c7cfa".to_owned(),
            background_color: "#f5f7ff".to_owned(),
        }
    }
}
