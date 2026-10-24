//! Shader source for render debugging. With `BO2MP_WGSL_DUMP` set to part of
//! a shader name, each pass whose vertex or pixel shader name contains it
//! has its WGSL written to `bo2mp-artifacts/wgsl/<vertex>__<pixel>.wgsl`.

use std::path::Path;
use std::sync::OnceLock;

pub fn dump_pass_wgsl(vertex_name: &str, pixel_name: &str, source: &str) {
    static WANTED: OnceLock<Option<String>> = OnceLock::new();
    let Some(wanted) = WANTED.get_or_init(|| std::env::var("BO2MP_WGSL_DUMP").ok()) else {
        return;
    };
    if !vertex_name.contains(wanted.as_str()) && !pixel_name.contains(wanted.as_str()) {
        return;
    }
    let dir = Path::new(crate::ARTIFACTS_DIR).join("wgsl");
    let path = dir.join(format!(
        "{}__{}.wgsl",
        file_safe(vertex_name),
        file_safe(pixel_name)
    ));
    match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, source)) {
        Ok(()) => crate::warn!(
            World,
            "BO2MP_WGSL_DUMP: {} bytes of {vertex_name} / {pixel_name} in {}",
            source.len(),
            path.display()
        ),
        Err(error) => crate::warn!(
            World,
            "BO2MP_WGSL_DUMP: cannot write {}: {error}",
            path.display()
        ),
    }
}

/// `name` with every character but ASCII letters and digits made `_`.
fn file_safe(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn shader_names_become_plain_file_names() {
        assert_eq!(super::file_safe("lit_r0c0/wc.hlsl"), "lit_r0c0_wc_hlsl");
    }
}
