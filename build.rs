// Embeds the app icon and version info into the Windows .exe (Explorer, taskbar, Alt+Tab).
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Gaza Man");
        res.set("FileDescription", "Gaza Man: a simple API client");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Couldn't embed the Windows icon: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
