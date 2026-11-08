//! Makes a thumbnail (or the icon when there is none) and saves it as PNG:
//! `cargo run -p rf-linux --example thumbnail -- <file> [size] [out.png]`

fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let file = a.next().expect("file");
    let size: u32 = a.next().and_then(|s| s.parse().ok()).unwrap_or(256);
    let out = a.next().unwrap_or_else(|| "thumbnail.png".into());
    let t = std::time::Instant::now();
    let b = rf_linux::thumbs::get_image(std::path::Path::new(&file), size, rf_linux::thumbs::Kind::Thumbnail)?;
    println!("{}x{} in {:?}", b.width, b.height, t.elapsed());
    let mut data = b.data;
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    image::RgbaImage::from_raw(b.width, b.height, data).expect("buffer").save(&out)?;
    println!("saved {out}");
    Ok(())
}
