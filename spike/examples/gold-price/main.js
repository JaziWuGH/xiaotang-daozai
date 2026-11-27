function refresh({ settings, storage, sources, now = Date.now() }) {
  const points = Array.isArray(storage?.points) ? storage.points : [];
  const gold = sources?.gold;
  if (gold?.status !== "ready") return { status: null, pinned: null, storage };

  let quote;
  try {
    quote = JSON.parse(gold.text);
  } catch {
    return { status: null, pinned: null, storage };
  }
  const price = quote?.xau?.price;
  const observed = Date.parse(quote?.price_as_of ?? quote?.data_state?.as_of ?? quote?.updated_at);
  const generated = Date.parse(quote?.updated_at);
  const latest = points[points.length - 1];
  if (quote?.xau?.currency !== "CNY" || quote?.xau?.unit !== "gram" ||
      !Number.isFinite(price) || price <= 0 || quote?.fx_stale ||
      quote?.data_state?.status !== "fresh" ||
      !Number.isFinite(generated) || now - generated > 5 * 60_000 ||
      !Number.isFinite(observed) || now - observed > 15 * 60_000 ||
      (latest && observed < latest.at)) {
    return { status: null, pinned: null, storage };
  }

  const threshold = Number(settings.threshold);
  const changed = latest && observed > latest.at;
  let alert;
  if (changed && Number.isFinite(threshold) && threshold > 0) {
    if (latest.price < threshold && price >= threshold) alert = `金价达到 ${threshold} 元/克`;
    if (latest.price > threshold && price <= threshold) alert = `金价跌至 ${threshold} 元/克`;
  }
  const history = latest?.at === observed ? points : [...points.slice(-47), { at: observed, price }];
  const value = price.toFixed(2) + " 元/克";
  return {
    notice: alert ? { id: "gold-" + observed, title: "国际现货金价", body: alert } : undefined,
    status: {
      state: "succeeded",
      label: "国际现货金价",
      metrics: [{ label: "人民币每克", value: price.toFixed(2) + " 元" }],
      series: history.length >= 2 ? history.map(point => point.price) : undefined,
      series_label: history.length >= 2 ? "人民币每克" : undefined,
      alert,
      observed_at: Math.floor(observed / 1000),
      expires_at: Math.floor((observed + 15 * 60_000) / 1000)
    },
    pinned: { text: "金价", value, icon: "coin" },
    storage: { points: history }
  };
}
