// Ink lifting off washi: when a clean-up settles, the list's last picture
// lets go of its ink along the paper's fibres, so the kinari ground and its
// grain show through where the cards were, and the settled page is inked in
// behind it through the same fibres. `sample_from` is the list as it was,
// `sample_to` the list as it is now, `nsh.progress` how far the lift is.

struct Uniforms {
    // 1 on the dark sheet, where the marks are light on dark paper.
    dark: f32,
}

fn hash21(p: vec2f) -> f32 {
    var q = fract(p * vec2f(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn value_noise(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2f(1.0, 0.0));
    let c = hash21(i + vec2f(0.0, 1.0));
    let d = hash21(i + vec2f(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// The sheet's make at a point given in points, 0..1: pulp clouds, long
// fibres laid mostly one way (as kozo settles on the screen) and short ones
// across them.
fn fibres(p: vec2f) -> f32 {
    let c = cos(0.42);
    let s = sin(0.42);
    let q = vec2f(c * p.x - s * p.y, s * p.x + c * p.y);
    let pulp = value_noise(p / 46.0) * 0.6 + value_noise(p / 17.0) * 0.4;
    let long_fibre = value_noise(vec2f(q.x / 70.0, q.y / 2.6));
    let short_fibre = value_noise(vec2f(q.y / 14.0, q.x / 1.8));
    return clamp(0.5 * pulp + 0.32 * long_fibre + 0.18 * short_fibre, 0.0, 1.0);
}

fn premultiplied(c: vec4f) -> vec4f {
    return vec4f(c.rgb * c.a, c.a);
}

fn shade(uv: vec2f) -> vec4f {
    let t = nsh.progress;
    let n = fibres(uv * nsh.size);

    // Ink is how far a mark sits from its paper: dark on the light sheet,
    // light on the dark one. Dense ink lets go a little before the wash
    // around it, so words loosen before the sheets under them.
    let was = sample_from(uv);
    let lum = dot(was.rgb, vec3f(0.2126, 0.7152, 0.0722));
    let ink = select(1.0 - lum, lum, uniforms.dark > 0.5) * was.a;
    let lets_go = max(0.02, 0.06 + 0.52 * n - 0.1 * ink);
    let held = 1.0 - smoothstep(lets_go, lets_go + 0.24, t);

    // What is still held drifts a point or two along the fibres as it
    // loosens, the way wet ink creeps before it lifts.
    let along = vec2f(0.91, 0.41) * ((1.0 - held) * (n - 0.5) * 3.0);
    let lifting = premultiplied(sample_from(uv + along / max(nsh.size, vec2f(1.0)))) * held;

    // The settled page inks in behind once the old one has mostly gone,
    // through the same fibres.
    let begins = 0.5 + 0.22 * (n - 0.5);
    let inked = smoothstep(begins, begins + 0.36, t);
    let now = premultiplied(sample_to(uv)) * inked;

    let mixed = now + lifting * (1.0 - now.a);
    if (mixed.a <= 0.0001) {
        return vec4f(0.0);
    }
    return vec4f(mixed.rgb / mixed.a, mixed.a);
}
