fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../kotodex/icons/kotodex.ico");
        winresource::WindowsResource::new()
            .set_icon("../kotodex/icons/kotodex.ico")
            .compile()
            .expect("stamping kotodex.ico into kotodex-server.exe");
    }
}
