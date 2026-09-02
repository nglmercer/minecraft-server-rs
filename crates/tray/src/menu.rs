use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};

pub(crate) struct TrayMenu {
    pub(crate) menu: Menu,
    pub(crate) open_panel: MenuItem,
    pub(crate) reset: MenuItem,
    pub(crate) exit: MenuItem,
}

pub(crate) fn build() -> Result<TrayMenu, String> {
    let menu = Menu::new();
    let title = MenuItem::new("MCP Panel", false, None);
    let open_panel = MenuItem::new("Open Panel", true, None);
    let reset = MenuItem::new("Reset Admin Password…", true, None);
    let exit = MenuItem::new("Exit MCP Panel", true, None);
    let separator = PredefinedMenuItem::separator();
    let separator2 = PredefinedMenuItem::separator();

    menu.append_items(&[&title, &separator, &open_panel, &reset, &separator2, &exit])
        .map_err(|error| error.to_string())?;

    Ok(TrayMenu {
        menu,
        open_panel,
        reset,
        exit,
    })
}
