// Light and dark changing through the paper: the new sheet soaks into the
// old one as a wash runs across washi, from the top-left corner where a
// reader starts, its front softened by the pulp's clouds. Every point is
// always one sheet or the other or both mixed: the window is never empty
// under it. `sample_from` is the window as it was, `sample_to` as it is
// now, `nsh.progress` how far the soak has run.

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

// The sheet's pulp at a point in points, 0..1: broad soft clouds, turned
// off the pixel grid so the noise's cells never line up square.
fn pulp(p: vec2f) -> f32 {
    let q = vec2f(0.83 * p.x - 0.56 * p.y, 0.56 * p.x + 0.83 * p.y);
    return value_noise(q / 140.0) * 0.6 + value_noise(q / 52.0 + 7.3) * 0.4;
}

fn shade(uv: vec2f) -> vec4f {
    let t = nsh.progress;
    let n = pulp(uv * nsh.size);
    // Where a point turns: mostly its place on the page, from the top-left
    // corner, with the pulp's clouds softening the front. 0..0.75, and the
    // front is 0.25 wide, so every point has turned by the end.
    let turns = 0.30 * n + 0.45 * (0.6 * uv.x + 0.4 * uv.y);
    let soaked = smoothstep(turns, turns + 0.25, t);
    let was = sample_from(uv);
    let now = sample_to(uv);
    return mix(was, now, soaked);
}
