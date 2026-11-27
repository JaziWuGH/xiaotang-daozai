function refresh(target, today) {
    const previous = lastDays();
    const days = target - today;
    saveDays(days);
    return previous;
}
function fault(which) {
    if (which === 1) { for (;;) {} }
    if (which === 2) { throw Error("演示异常"); }
    if (which === 3) { new ArrayBuffer(16 * 1024 * 1024); }
    if (which === 4) { return request("https://denied.example/test"); }
    if (which === 5) { return typeof require === "undefined" && typeof std === "undefined" && typeof os === "undefined" ? -1 : 0; }
    return request("https://allowed.example/test");
}
