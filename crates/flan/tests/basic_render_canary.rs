// Render infrastructure canary test.
#![cfg(not(target_arch = "wasm32"))]

use flan::test::lib::shader::canary;

#[test]
fn render_canary_all_black_pixels() {
    let Some(frame) = canary::render() else {
        return;
    };

    let expected: [u8; 4] = [0, 0, 0, 255];
    let total = (frame.width * frame.height) as usize;

    let bad_pixels: Vec<(u32, u32, [u8; 4])> = frame
        .pixels
        .chunks_exact(4)
        .enumerate()
        .filter_map(|(i, p)| {
            let pixel = [p[0], p[1], p[2], p[3]];
            if pixel != expected {
                let x = (i as u32) % frame.width;
                let y = (i as u32) / frame.width;
                Some((x, y, pixel))
            } else {
                None
            }
        })
        .collect();

    if !bad_pixels.is_empty() {
        let n = bad_pixels.len();
        let first_five: Vec<_> = bad_pixels.iter().take(5).collect();
        panic!(
            "render canary: {n}/{total} pixels are not [0,0,0,255] first bad pixels (x, y, [R,G,B,A]): {first_five:#?}  This means the Bevy headless render harness is broken on this platform. Check camera RenderTarget setup and UiMaterial pipeline setup.",
        );
    }
}
