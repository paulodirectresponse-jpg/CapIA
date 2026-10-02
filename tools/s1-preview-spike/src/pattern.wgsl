// Padrão de teste do S1. Saída NÃO-sRGB (valores crus) para que a leitura de pixels da tela seja exata.
struct U { frame: u32, w: f32, h: f32, t: f32 };
@group(0) @binding(0) var<uniform> u: U;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let px = p.xy;
    var col = vec3f(0.08, 0.16, 0.30) + vec3f(px.x / u.w * 0.20, px.y / u.h * 0.20, 0.0);
    // barra amarela móvel (indica animação contínua)
    let bar_x = (u.t * 240.0) % u.w;
    if (abs(px.x - bar_x) < 6.0) { col = vec3f(1.0, 0.9, 0.0); }
    // zona de sonda: cinza plano 128 em (0.05..0.25, 0.05..0.20) — referência para transparência/overlay
    if (px.x >= 0.05 * u.w && px.x < 0.25 * u.w && px.y >= 0.05 * u.h && px.y < 0.20 * u.h) {
        col = vec3f(128.0 / 255.0);
    }
    // faixa de código (canto inferior esquerdo): 18 blocos de 24 px.
    // bloco 0 = branco (calibração), blocos 1..16 = bits do frame (LSB primeiro), bloco 17 = preto (calibração)
    let blk = 24.0;
    if (px.y >= u.h - blk && px.x < 18.0 * blk) {
        let i = u32(px.x / blk);
        var v = 0.0;
        if (i == 0u) { v = 1.0; }
        else if (i <= 16u) { v = f32((u.frame >> (i - 1u)) & 1u); }
        col = vec3f(v);
    }
    return vec4f(col, 1.0);
}
