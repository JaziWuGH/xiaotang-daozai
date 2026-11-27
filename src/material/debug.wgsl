fn material(i : MaterialIn) -> vec4<f32> {
    let fill = 1.0 - smoothstep(-0.6, 0.6, body_sdf(i.px));
    // 逐像素读取底片，避免在有效纹理边缘插值，方便核对屏幕坐标。
    let pixel = clamp(vec2<i32>(i.px), vec2<i32>(0), vec2<i32>(frame.tex.zw) - 1);
    let desktop = textureLoad(backdrop, pixel, 0).rgb;
    var border = 0.0;
    for (var index = 0u; index < min(frame.count.x, 16u); index += 1u) {
        let rect = frame.ctl_rect[index];
        let shape = frame.ctl_shape[index];
        if (shape.y > 0.5) {
            let half_size = rect.zw * 0.5;
            let radius = min(shape.x, min(half_size.x, half_size.y));
            let distance = rr_sdf(i.px - rect.xy - half_size, half_size, radius);
            border = max(border, 1.0 - smoothstep(0.35, 1.25, abs(distance)));
        }
    }
    let color = vec3<f32>(0.0, 0.9, 0.8);
    return vec4<f32>(mix(desktop * fill, color, border), mix(fill, 1.0, border));
}
