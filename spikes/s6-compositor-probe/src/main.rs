//! S6 probe: does the OpenCut Classic compositor behave correctly and scale acceptably?
use compositor::*;
use gpu::{wgpu, GpuContext};
use std::time::Instant;

type Px = [u8; 4]; // BGRA (Bgra8Unorm on Vulkan)
fn bgra(r: u8, g: u8, b: u8, a: u8) -> Px { [b, g, r, a] }

fn upload(ctx: &GpuContext, w: u32, h: u32, f: impl Fn(u32, u32) -> Px) -> wgpu::Texture {
    let tex = ctx.create_render_texture(w, h, "probe-src");
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h { for x in 0..w { data.extend_from_slice(&f(x, y)); } }
    ctx.queue().write_texture(
        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    tex
}

fn readback(ctx: &GpuContext, tex: &wgpu::Texture, w: u32, h: u32) -> Vec<Px> {
    let bpr = (w * 4 + 255) / 256 * 256;
    let buf = ctx.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("rb"), size: (bpr * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
    });
    let mut enc = ctx.device().create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    ctx.queue().submit([enc.finish()]);
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
    ctx.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = slice.get_mapped_range();
    let mut out = Vec::new();
    for y in 0..h { for x in 0..w {
        let o = (y * bpr + x * 4) as usize;
        out.push([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    } }
    out
}

fn layer(id: &str, cx: f32, cy: f32, w: f32, h: f32, rot: f32, opacity: f32, blend: BlendMode) -> LayerDescriptor {
    LayerDescriptor {
        texture_id: id.into(),
        transform: QuadTransformDescriptor { center_x: cx, center_y: cy, width: w, height: h, rotation_degrees: rot, flip_x: false, flip_y: false },
        opacity, blend_mode: blend, effect_pass_groups: vec![], mask: None,
    }
}

fn frame(w: u32, h: u32, items: Vec<LayerDescriptor>) -> FrameDescriptor {
    FrameDescriptor { width: w, height: h, clear: CanvasClearDescriptor { color: [0.0, 0.0, 0.0, 1.0] },
        items: items.into_iter().map(FrameItemDescriptor::Layer).collect() }
}

struct Check { name: &'static str, ok: bool, detail: String }
fn near(a: Px, e: Px, tol: i32) -> bool { (0..4).all(|i| (a[i] as i32 - e[i] as i32).abs() <= tol) }
fn rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    let kb: f64 = s.lines().find(|l| l.starts_with("VmHWM")).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    kb / 1024.0
}

fn main() {
    let ctx = pollster::block_on(GpuContext::new()).expect("gpu init");
    let info = ctx.adapter().get_info();
    println!("ADAPTER: {} backend={:?} type={:?} format={:?}", info.name, info.backend, info.device_type, ctx.texture_format());
    let mut comp = Compositor::new(&ctx);
    let (w, h) = (256u32, 256u32);
    let mut checks: Vec<Check> = vec![];

    // textures
    comp.upsert_texture("blue".into(), upload(&ctx, 8, 8, |_, _| bgra(0, 0, 255, 255)));
    comp.upsert_texture("red".into(), upload(&ctx, 8, 8, |_, _| bgra(255, 0, 0, 255)));
    comp.upsert_texture("halves".into(), upload(&ctx, 8, 8, |x, _| if x < 4 { bgra(0, 255, 0, 255) } else { bgra(255, 255, 255, 255) }));
    comp.upsert_texture("gray".into(), upload(&ctx, 8, 8, |_, _| bgra(200, 100, 50, 255)));
    comp.upsert_texture("mul".into(), upload(&ctx, 8, 8, |_, _| bgra(100, 200, 255, 255)));
    comp.upsert_texture("maskhalf".into(), upload(&ctx, 8, 8, |x, _| if x < 4 { bgra(255, 255, 255, 255) } else { bgra(0, 0, 0, 0) }));
    comp.upsert_texture("whitesq".into(), upload(&ctx, 8, 8, |_, _| bgra(255, 255, 255, 255)));

    let bg = layer("blue", 128.0, 128.0, 256.0, 256.0, 0.0, 1.0, BlendMode::Normal);
    let mut render = |comp: &mut Compositor, f: &FrameDescriptor| -> Vec<Px> {
        let t = comp.render_frame_to_texture(&ctx, f).expect("render");
        readback(&ctx, &t, w, h)
    };
    let at = |p: &Vec<Px>, x: u32, y: u32| p[(y * w + x) as usize];

    // C1: normal blend, 50% red over blue => ~purple; outside quad stays blue
    let p = render(&mut comp, &frame(w, h, vec![bg.clone(), layer("red", 128.0, 128.0, 100.0, 100.0, 0.0, 0.5, BlendMode::Normal)]));
    checks.push(Check { name: "normal 50% red over blue (center)", ok: near(at(&p,128,128), bgra(128,0,128,255), 3), detail: format!("{:?} want {:?}", at(&p,128,128), bgra(128,0,128,255)) });
    checks.push(Check { name: "outside quad stays background", ok: near(at(&p,10,10), bgra(0,0,255,255), 1), detail: format!("{:?}", at(&p,10,10)) });

    // C2: rotation 45deg of 100x100 square: (177,177) outside, (188,128) inside
    let p = render(&mut comp, &frame(w, h, vec![bg.clone(), layer("red", 128.0, 128.0, 100.0, 100.0, 45.0, 1.0, BlendMode::Normal)]));
    checks.push(Check { name: "rotation: unrotated corner now background", ok: near(at(&p,177,177), bgra(0,0,255,255), 2), detail: format!("{:?}", at(&p,177,177)) });
    checks.push(Check { name: "rotation: diamond tip is red", ok: near(at(&p,188,128), bgra(255,0,0,255), 2), detail: format!("{:?}", at(&p,188,128)) });

    // C3: flip_x
    let mut fl = layer("halves", 128.0, 128.0, 128.0, 128.0, 0.0, 1.0, BlendMode::Normal);
    let p0 = render(&mut comp, &frame(w, h, vec![bg.clone(), fl.clone()]));
    fl.transform.flip_x = true;
    let p1 = render(&mut comp, &frame(w, h, vec![bg.clone(), fl]));
    checks.push(Check { name: "flip_x swaps halves", ok: near(at(&p0,80,128), bgra(0,255,0,255),2) && near(at(&p1,80,128), bgra(255,255,255,255),2), detail: format!("{:?} -> {:?}", at(&p0,80,128), at(&p1,80,128)) });

    // C4: multiply blend  (200,100,50) * (100,200,255)/255
    let p = render(&mut comp, &frame(w, h, vec![layer("gray",128.0,128.0,256.0,256.0,0.0,1.0,BlendMode::Normal), layer("mul",128.0,128.0,256.0,256.0,0.0,1.0,BlendMode::Multiply)]));
    let e = bgra((200u32*100/255) as u8, (100u32*200/255) as u8, (50u32*255/255) as u8, 255);
    checks.push(Check { name: "multiply blend", ok: near(at(&p,128,128), e, 3), detail: format!("{:?} want {:?}", at(&p,128,128), e) });

    // C5: mask (white-left/transparent-right): what do we get? (semantics probe)
    let mut ml = layer("red", 128.0, 128.0, 256.0, 256.0, 0.0, 1.0, BlendMode::Normal);
    ml.mask = Some(LayerMaskDescriptor { texture_id: "maskhalf".into(), feather: 0.0, inverted: false });
    let p = render(&mut comp, &frame(w, h, vec![bg.clone(), ml.clone()]));
    println!("MASK probe (no feather): left={:?} right={:?}", at(&p,64,128), at(&p,192,128));
    checks.push(Check { name: "mask applies (left red / right background)", ok: near(at(&p,64,128), bgra(255,0,0,255),3) && near(at(&p,192,128), bgra(0,0,255,255),3), detail: format!("{:?} | {:?}", at(&p,64,128), at(&p,192,128)) });
    ml.mask.as_mut().unwrap().feather = 20.0;
    let p = render(&mut comp, &frame(w, h, vec![bg.clone(), ml]));
    let mid = at(&p,128,128);
    checks.push(Check { name: "mask feather yields soft edge at boundary", ok: mid != bgra(255,0,0,255) && mid != bgra(0,0,255,255), detail: format!("boundary {:?}", mid) });

    // C6: gaussian blur effect on a small white square over black
    let mut bl = layer("whitesq", 128.0, 128.0, 40.0, 40.0, 0.0, 1.0, BlendMode::Normal);
    // NOTE: EffectUniformValueDescriptor is not re-exported from compositor's lib.rs -> build via serde (API gap found by this spike)
    let mk = |dir: [f32; 2]| -> EffectPassDescriptor { serde_json::from_value(serde_json::json!({"shader":"gaussian-blur","uniforms":{"u_sigma":6.0,"u_step":1.0,"u_direction":dir}})).unwrap() };
    bl.effect_pass_groups = vec![vec![mk([1.0, 0.0])], vec![mk([0.0, 1.0])]];
    let blank = layer("blue", 128.0, 128.0, 0.0001, 0.0001, 0.0, 0.0, BlendMode::Normal);
    match comp.render_frame_to_texture(&ctx, &frame(w, h, vec![blank, bl])) {
        Ok(t) => { let p = readback(&ctx, &t, w, h); let outside = at(&p,152,128); // 4px beyond edge (edge at 148)
            checks.push(Check { name: "gaussian blur softens edge (pixel 4px outside > 0)", ok: outside[0] > 5, detail: format!("{:?}", outside) }); }
        Err(e) => checks.push(Check { name: "gaussian blur effect", ok: false, detail: format!("error: {e}") }),
    }

    println!("\nCORRECTNESS");
    for c in &checks { println!("  [{}] {} — {}", if c.ok {"PASS"} else {"FAIL"}, c.name, c.detail); }

    // PERF/MEMORY SCALING at 1080p (software Vulkan: ABSOLUTE times are meaningless, scaling is not)
    println!("\nSCALING (1920x1080, quads 400x300, llvmpipe)  baseline RSS {:.0} MB", rss_mb());
    let (bw, bh) = (1920u32, 1080u32);
    let mut c2 = Compositor::new(&ctx);
    c2.upsert_texture("q".into(), upload(&ctx, 64, 64, |x, y| bgra((x*4) as u8, (y*4) as u8, 128, 255)));
    for n in [1usize, 10, 50, 200] {
        let items: Vec<_> = (0..n).map(|i| layer("q", 200.0 + (i % 20) as f32 * 80.0, 150.0 + (i / 20 % 6) as f32 * 150.0, 400.0, 300.0, (i % 7) as f32 * 3.0, 0.8, BlendMode::Normal)).collect();
        let f = frame(bw, bh, items);
        let _ = c2.render_frame_to_texture(&ctx, &f).unwrap(); ctx.device().poll(wgpu::PollType::wait_indefinitely()).unwrap(); // warm-up/alloc
        let t0 = Instant::now(); let reps = 3;
        for _ in 0..reps { let t = c2.render_frame_to_texture(&ctx, &f).unwrap(); ctx.device().poll(wgpu::PollType::wait_indefinitely()).unwrap(); drop(t); }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / reps as f64;
        println!("  layers={:<4} frame={:>8.1} ms  per-layer={:>6.2} ms  peak RSS={:.0} MB", n, ms, ms / n as f64, rss_mb());
    }
}
