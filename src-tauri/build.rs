fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("icons/icon.ico");
        resource.set("FileDescription", "Voxen");
        resource.set("ProductName", "Voxen");
        resource.set("OriginalFilename", "Voxen.exe");
        resource.compile().expect("windows resources");
    }
}
