// 液态玻璃，移植自轻九九九（@cayww）的 PR #235。
const LUMA : vec3<f32> = vec3<f32>(0.299, 0.587, 0.114);
const NEUTRAL_TINT : vec3<f32> = vec3<f32>(0.62, 0.66, 0.74);
const ROLE_ISLAND_LYRICS : u32 = 7u;
const ROLE_SEEK : u32 = 10u;

fn hash21(p : vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(0.1031, 0.1030));
    q = q + vec2<f32>(dot(q, q.yx + vec2<f32>(33.33)));
    return fract((q.x + q.y) * q.x);
}

// 通过十字五点采样模糊底片，红蓝通道沿法线反向采样。
fn tap5(p : vec2<f32>, dispersion : vec2<f32>, blur : f32) -> vec3<f32> {
    var offsets = array<vec2<f32>, 5>(
        vec2<f32>(0.0), vec2<f32>(blur, 0.0), vec2<f32>(-blur, 0.0),
        vec2<f32>(0.0, blur), vec2<f32>(0.0, -blur)
    );
    var color = vec3<f32>(0.0);
    for (var i = 0u; i < 5u; i = i + 1u) {
        let q = p + offsets[i];
        color += vec3<f32>(
            sample_backdrop(q + dispersion).r,
            sample_backdrop(q).g,
            sample_backdrop(q - dispersion).b
        );
    }
    return color / 5.0;
}

fn rounded_normal(p : vec2<f32>, half_size : vec2<f32>, radius : f32) -> vec2<f32> {
    let dx = vec2<f32>(1.0, 0.0);
    let dy = vec2<f32>(0.0, 1.0);
    let gx = rr_sdf(p + dx, half_size, radius) - rr_sdf(p - dx, half_size, radius);
    let gy = rr_sdf(p + dy, half_size, radius) - rr_sdf(p - dy, half_size, radius);
    return normalize(vec2<f32>(gx, gy) + vec2<f32>(1e-6));
}

fn corner_weight(local : vec2<f32>, half_size : vec2<f32>, radius : f32) -> f32 {
    let corner = clamp((abs(local) - (half_size - radius)) / max(radius, 1.0), vec2<f32>(0.0), vec2<f32>(1.0));
    return mix(0.60, 1.0, corner.x * corner.y);
}

struct Optics {
    refraction : f32,
    dispersion : f32,
    blur : f32,
    edge_width : f32,
    edge_opacity : f32,
};

struct ControlHit {
    index : u32,
    local : vec2<f32>,
    half_size : vec2<f32>,
    radius : f32,
    distance : f32,
};

fn pick_control(p : vec2<f32>) -> ControlHit {
    let count = min(frame.count.x, 16u);
    var hit = ControlHit(count, vec2<f32>(0.0), vec2<f32>(1.0), 0.0, 0.0);
    // 宿主按面积从大到小提供控件，较小的玻璃片覆盖较大的底板。
    for (var index = 0u; index < count; index = index + 1u) {
        let ctrl = frame.ctl_shape[index];
        if (ctrl.y < 0.5) { continue; }
        let rect = frame.ctl_rect[index];
        let spring = frame.ctl_spring[index];
        let half_size = max(rect.zw * (0.5 * spring.z) - vec2<f32>(0.5), vec2<f32>(1.0));
        let radius = clamp(ctrl.x * spring.z - 0.5, 0.0, min(half_size.x, half_size.y));
        let local = p - rect.xy - rect.zw * 0.5 - spring.xy;
        let distance = rr_sdf(local, half_size, radius);
        if (distance < 0.0) {
            hit = ControlHit(index, local, half_size, radius, distance);
        }
    }
    return hit;
}

fn control_glass(p : vec2<f32>, hit : ControlHit, mat : f32, light : vec2<f32>, edge_bg : vec3<f32>, optics : Optics) -> vec4<f32> {
    let local = hit.local;
    let hs = hit.half_size;
    let distance = hit.distance;
    let normal = rounded_normal(local, hs, hit.radius);
    let short = max(min(hs.x, hs.y), 1.0);
    let band = clamp(short * 1.30, 10.0, 72.0);
    let t = clamp(-distance / band, 0.0, 1.0);
    let k = pow(1.0 - pow(t, 0.30), 2.0);
    let size = frame.ctl_rect[hit.index].zw / frame.env.z;
    let role = u32(frame.ctl_spring[hit.index].w);
    var lens = 1.12;
    switch role {
        case ROLE_ISLAND_LYRICS: { lens = 1.06; }
        case ROLE_SEEK: { lens = 1.0 + 0.9 * frame.ctl_shape[hit.index].w; }
        default: {}
    }
    let short_side = min(size.x, size.y) / select(1.0, lens, role == ROLE_SEEK);
    let refract = 0.55 * clamp(short_side / 70.0, 0.30, 1.0);
    let chroma = optics.dispersion * refract * corner_weight(local, hs, hit.radius);
    let bp = p - local * (1.0 - 1.0 / lens) - normal * k * optics.refraction * refract;

    var col = tap5(bp, -normal * k * chroma, optics.blur * 0.80 + k * 1.2);
    let lum = dot(col, LUMA);
    col *= 1.0 + mix(0.045, -0.035, smoothstep(0.42, 0.88, lum));
    col = mix(col, vec3<f32>(0.30), smoothstep(0.48, 0.96, dot(edge_bg, LUMA)) * 0.16 * mat);
    col = col * 1.010 + vec3<f32>(0.004, 0.006, 0.010);
    col = mix(col, col * 0.58 + vec3<f32>(0.010, 0.014, 0.020), (1.0 - mat) * 0.70);
    let top = 1.0 - smoothstep(0.0, max(hs.y * 2.0, 1.0), local.y + hs.y);
    col += vec3<f32>(0.06, 0.08, 0.11) * top * 0.16;

    let tq = (distance + 1.7) / 0.80;
    let bevel = exp(-tq * tq);
    col *= 1.0 - bevel * 0.10;
    let bleed = exp(-max(-distance, 0.0) / clamp(short * 0.45, 5.0, 22.0)) * (1.0 - bevel * 0.5);
    let lobe = pow(max(dot(normal, light), 0.0), 1.5);
    col += vec3<f32>(0.62, 0.86, 0.96) * bleed * (lobe * 0.15 + 0.055);

    let sigma = max(optics.edge_width, 0.5) * 0.42;
    let lq = (distance + sigma * 0.65) / sigma;
    let line = exp(-lq * lq) * mix(0.55, 1.0, pow(max(dot(normal, light), 0.0), 1.3));
    let edge = clamp(line * optics.edge_opacity, 0.0, 1.0);
    col = mix(col, vec3<f32>(1.0), edge);
    let aa = 1.0 - smoothstep(-1.0, 1.0, distance);
    return vec4<f32>(col, max(aa, edge));
}

fn material(i : MaterialIn) -> vec4<f32> {
    let p = i.px;
    let distance = body_sdf(p);
    if (distance > 2.75 || body_sdf_exact(p) > 2.0) { return vec4<f32>(0.0); }
    let edge_bg = sample_backdrop(p);

    // 位置和法线使用物理像素，清单的长度参数使用逻辑点。
    let ppp = frame.env.z;
    let optics = Optics(param(0u) * ppp, param(1u) * ppp, param(2u) * ppp, param(4u) * ppp, param(5u));
    let mat = clamp(frame.motion.y, 0.0, 1.0);
    let center = frame.body.xy + frame.body.zw * 0.5;
    let local = p - center;
    let half_size = max(frame.body.zw * 0.5 - vec2<f32>(3.0), vec2<f32>(2.0));
    let radius = clamp(frame.shape.x - 3.0, 0.0, min(half_size.x, half_size.y));
    let normal = rounded_normal(local, half_size, radius);
    let to_pointer = select(frame.shape.zw - center, vec2<f32>(0.0), all(frame.shape.zw == vec2<f32>(-1.0)));
    let pointer_distance = length(to_pointer);
    let swung = to_pointer / max(pointer_distance, 1e-3);
    let light = normalize(mix(
        normalize(vec2<f32>(-0.45, -0.65)), vec2<f32>(swung.x, min(swung.y, -0.18)),
        smoothstep(3.0, 48.0, pointer_distance)
    ) + vec2<f32>(1e-6));

    // 展开和收起都按宿主平滑后的速度增强折射。
    let morph = max(param(6u), 0.0) * clamp(frame.motion.z, 0.0, 1.0);
    let band = max(param(3u) * ppp, 1.0) * (1.0 + morph * 0.08);
    let t = clamp(-distance / band, 0.0, 1.0);
    let k = pow(1.0 - pow(t, 0.30), 2.0);
    let strength = optics.refraction * (1.0 + morph * 0.15);
    let chroma = optics.dispersion * corner_weight(local, half_size, radius);
    var col = tap5(p - normal * k * strength, -normal * k * chroma, optics.blur + k * 1.6);
    col = col * 1.004 + vec3<f32>(0.002, 0.003, 0.006);
    let top = 1.0 - smoothstep(0.0, max(frame.body.w, 1.0), p.y - frame.body.y);
    col += vec3<f32>(0.05, 0.07, 0.10) * top * 0.10;

    let lum = dot(col, LUMA);
    col *= 1.0 + mix(0.045, -0.035, smoothstep(0.42, 0.88, lum));
    col = mix(col, col * 0.58 + vec3<f32>(0.010, 0.014, 0.020), (1.0 - mat) * 0.70);
    col = mix(col, vec3<f32>(0.96, 0.97, 1.0), mat * 0.02);
    let scrim = smoothstep(0.48, 0.96, dot(edge_bg, LUMA)) * 0.16 * mat;
    col = mix(col, vec3<f32>(0.30), scrim);
    col = mix(vec3<f32>(dot(col, LUMA)), col, 1.06);

    // 倒角暗带与青绿色导光，迎光侧和背光侧各有一层高光。
    let tq = (distance + 2.2) / 0.9;
    let bevel = exp(-tq * tq);
    col *= 1.0 - bevel * 0.08;
    let depth = max(-distance, 0.0);
    let bleed = exp(-depth / 16.0) * (1.0 - bevel * 0.55);
    let path = exp(-depth / 7.0);
    col *= mix(vec3<f32>(1.0), vec3<f32>(0.90, 1.0, 0.96), path * 0.65);
    let absorb = 1.0 - exp(-depth / 10.0);
    let tint = mix(vec3<f32>(0.88, 0.93, 1.0), vec3<f32>(0.52, 0.90, 0.80), absorb);
    let lobe_a = pow(max(dot(normal, light), 0.0), 1.6);
    let lobe_b = pow(max(dot(normal, -light), 0.0), 1.2);
    col += tint * bleed * (lobe_a * 0.16 + lobe_b * 0.085 + 0.050);
    col += NEUTRAL_TINT * 0.05 * bleed;

    let sigma = max(optics.edge_width, 0.3) * 0.365;
    let lq = (distance + sigma * 0.55) / sigma;
    let line = exp(-lq * lq) * mix(0.40, 1.0, pow(max(dot(normal, light), 0.0), 1.5));
    let gq = (distance + 1.0) / max(3.2 * optics.edge_width / 1.5, 0.8);
    let glow = exp(-gq * gq) * 0.04;
    let line_weight = optics.edge_opacity * mix(0.69, 1.02, smoothstep(0.60, 0.92, lum));
    col = mix(col, vec3<f32>(1.0), clamp(line * line_weight + glow, 0.0, 1.0));

    let hit = pick_control(p);
    if (hit.index < frame.count.x) {
        let control = control_glass(p, hit, mat, light, edge_bg, optics);
        col = mix(col, control.rgb, control.a);
    }

    let aa = 1.0 - smoothstep(-1.2, 1.2, distance);
    let coverage = clamp(max(aa, line * optics.edge_opacity), 0.0, 1.0);
    let dither = (hash21(p) - 0.5) * (1.7 / 255.0);
    let color = mix(edge_bg, col, coverage) + vec3<f32>(dither);
    return vec4<f32>(color * coverage, coverage);
}
