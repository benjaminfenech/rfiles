//! Decodes an image the way DirectLook does and prints its size, timing and EXIF field count:
//! `cargo run -p rf-linux --example look_decode -- <file> [max side] [out.png]`

fn main() -> anyhow::Result<()> {
    use rf_linux::look::decode;
    let mut a = std::env::args().skip(1);
    let file = a.next().expect("file");
    let max: u32 = a.next().and_then(|s| s.parse().ok()).unwrap_or(8192);
    let path = std::path::Path::new(&file);
    let t = std::time::Instant::now();
    println!(
        "header size: {:?} in {:?}",
        decode::dimensions(path),
        t.elapsed()
    );
    let t = std::time::Instant::now();
    let d = decode::decode_full(path, max)?;
    let f = &d.frames[0].0;
    println!(
        "decoded {}x{} (shown {}x{}), {} EXIF fields, in {:?}",
        d.width,
        d.height,
        f.width,
        f.height,
        d.exif.len(),
        t.elapsed()
    );
    if let Some(out) = a.next() {
        let mut data = f.data.clone();
        for px in data.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        image::RgbaImage::from_raw(f.width, f.height, data)
            .expect("buffer")
            .save(&out)?;
        println!("saved {out}");
    }
    Ok(())
}

