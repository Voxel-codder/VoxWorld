fn main() {
    #[cfg(windows)]
    {
        let icon = std::path::Path::new("../assets/voxygen/logo.ico");
        if icon.exists() {
            let mut res = winres::WindowsResource::new();
            res.set_icon(icon.to_str().unwrap());
            res.set("ProductName", "Vox World Launcher");
            res.set("FileDescription", "Vox World Launcher");
            let _ = res.compile();
        }
    }
}
