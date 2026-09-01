//! Image comparison tool: max/mean absolute error, differing-pixel count, and
//! SSIM. Used both for correctness gates (BVH vs linear scan must match) and
//! for the accuracy benchmark against a reference renderer.

fn load(path: &str) -> (u32, u32, Vec<u8>) {
    let img = image::open(path)
        .unwrap_or_else(|e| panic!("cannot open {}: {}", path, e))
        .to_rgb8();
    (img.width(), img.height(), img.into_raw())
}

/// Rec.709 luma, on sRGB-encoded values (SSIM is conventionally computed in
/// display space, not linear).
fn luma(p: &[u8]) -> f64 {
    0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64
}

/// Global SSIM over 8x8 tiles, averaged. Standard constants for an 8-bit range.
fn ssim(a: &[f64], b: &[f64], w: usize, h: usize) -> f64 {
    const C1: f64 = 6.5025; // (0.01*255)^2
    const C2: f64 = 58.5225; // (0.03*255)^2
    const WIN: usize = 8;

    let mut total = 0.0;
    let mut n = 0usize;
    let mut y = 0;
    while y + WIN <= h {
        let mut x = 0;
        while x + WIN <= w {
            let (mut ma, mut mb) = (0.0, 0.0);
            for j in 0..WIN {
                for i in 0..WIN {
                    ma += a[(y + j) * w + x + i];
                    mb += b[(y + j) * w + x + i];
                }
            }
            let cnt = (WIN * WIN) as f64;
            ma /= cnt;
            mb /= cnt;

            let (mut va, mut vb, mut cov) = (0.0, 0.0, 0.0);
            for j in 0..WIN {
                for i in 0..WIN {
                    let da = a[(y + j) * w + x + i] - ma;
                    let db = b[(y + j) * w + x + i] - mb;
                    va += da * da;
                    vb += db * db;
                    cov += da * db;
                }
            }
            va /= cnt - 1.0;
            vb /= cnt - 1.0;
            cov /= cnt - 1.0;

            let s = ((2.0 * ma * mb + C1) * (2.0 * cov + C2))
                / ((ma * ma + mb * mb + C1) * (va + vb + C2));
            total += s;
            n += 1;
            x += WIN;
        }
        y += WIN;
    }
    if n == 0 {
        1.0
    } else {
        total / n as f64
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        eprintln!("usage: compare <a.png> <b.png> [--diff out.png]");
        std::process::exit(2);
    }

    let (w1, h1, p1) = load(&a[0]);
    let (w2, h2, p2) = load(&a[1]);
    if (w1, h1) != (w2, h2) {
        eprintln!("size mismatch: {}x{} vs {}x{}", w1, h1, w2, h2);
        std::process::exit(1);
    }

    let mut max_err = 0u32;
    let mut sum_err = 0u64;
    let mut ndiff = 0usize;
    let mut diff_img = vec![0u8; p1.len()];

    for i in (0..p1.len()).step_by(3) {
        let mut pix_max = 0u32;
        for c in 0..3 {
            let d = (p1[i + c] as i32 - p2[i + c] as i32).unsigned_abs();
            sum_err += d as u64;
            pix_max = pix_max.max(d);
        }
        if pix_max > 0 {
            ndiff += 1;
        }
        max_err = max_err.max(pix_max);
        // Amplify 8x so single-level differences are visible.
        let v = (pix_max * 8).min(255) as u8;
        diff_img[i] = v;
        diff_img[i + 1] = v;
        diff_img[i + 2] = v;
    }

    let npx = (w1 * h1) as usize;
    let la: Vec<f64> = p1.chunks(3).map(luma).collect();
    let lb: Vec<f64> = p2.chunks(3).map(luma).collect();
    let s = ssim(&la, &lb, w1 as usize, h1 as usize);

    println!("  size          {}x{}  ({} px)", w1, h1, npx);
    println!(
        "  differing px  {} ({:.4}%)",
        ndiff,
        100.0 * ndiff as f64 / npx as f64
    );
    println!("  max abs err   {} / 255", max_err);
    println!(
        "  mean abs err  {:.5} / 255",
        sum_err as f64 / (npx * 3) as f64
    );
    let mean_a: f64 = la.iter().sum::<f64>() / npx as f64;
    let mean_b: f64 = lb.iter().sum::<f64>() / npx as f64;
    println!(
        "  mean luma     {:.2} vs {:.2}  (ratio {:.3})",
        mean_a,
        mean_b,
        mean_a / mean_b
    );
    println!("  SSIM          {:.6}", s);

    if let Some(i) = a.iter().position(|x| x == "--diff") {
        if let Some(out) = a.get(i + 1) {
            image::save_buffer(out, &diff_img, w1, h1, image::ColorType::Rgb8).ok();
            println!("  diff map      {} (8x amplified)", out);
        }
    }
}
