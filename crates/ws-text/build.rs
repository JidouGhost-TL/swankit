use std::path::Path;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(private_text_definitions)");
    let private = [
        "data/word-dialog-1.json",
        "data/word-dialog-2.json",
        "data/word-mask-1.json",
    ];
    let mut present = 0;
    for path in private {
        println!("cargo:rerun-if-changed={path}");
        present += usize::from(Path::new(path).is_file());
    }
    if present == private.len() {
        println!("cargo:rustc-cfg=private_text_definitions");
    } else {
        assert_eq!(
            present, 0,
            "private text definitions must be supplied together"
        );
    }
}
