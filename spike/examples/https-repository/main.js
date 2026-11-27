function refresh({ source }) {
  if (source.status === "unconfigured") return { card: "请授权 HTTPS 数据源" };
  if (source.status === "offline") return { card: "仓库数据暂不可用" };
  const repository = JSON.parse(source.text);
  return { card: "Rust · " + repository.stargazers_count + " 星标" };
}
