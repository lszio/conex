// 模型服务视图（P10）：节点直连读取的 LLM 提供者投影。health 来自节点对
// 上游的真实探测（live/stale/unreachable），不是前端推断；模型列表是节点
// 配置白名单过滤后的上游事实，tools=unknown 表示上游未声明，不冒充支持。

import { Badge } from "@/components/ui/card";
import type { LlmModelInfo, LlmProviderInfo, Workbench } from "@/hooks/use-workbench";

function HealthBadge({ health }: { health: string }) {
  const variant = health === "live" ? "success" : health === "stale" ? "secondary" : "destructive";
  const label = health === "live" ? "在线" : health === "stale" ? "过期" : "不可达";
  return <Badge variant={variant}>{label}</Badge>;
}

function ModelRow({ model }: { model: LlmModelInfo }) {
  return (
    <div className="flex items-center justify-between border-b border-[var(--color-border)] py-1.5 last:border-0">
      <span className="font-mono text-sm">{model.id}</span>
      <span className="flex items-center gap-2 text-xs text-[var(--color-muted-foreground)]">
        {model.ownedBy ? <span>{model.ownedBy}</span> : null}
        <span>{model.tools === "supported" ? "tools" : model.tools === "unknown" ? "tools?" : "无 tools"}</span>
      </span>
    </div>
  );
}

function ProviderCard({ provider }: { provider: LlmProviderInfo }) {
  return (
    <section className="rounded-lg border border-[var(--color-border)] bg-[var(--color-card)] p-4">
      <div className="mb-2 flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold">{provider.id}</h2>
        <HealthBadge health={provider.health} />
      </div>
      <p className="font-mono text-xs text-[var(--color-muted-foreground)]">{provider.baseUrl}</p>
      <p className="mt-1 text-xs text-[var(--color-muted-foreground)]">
        {provider.checkedAtMs
          ? `上次探测 ${new Date(Number(provider.checkedAtMs)).toLocaleTimeString()}`
          : "尚未探测"}
        {provider.latencyMs !== undefined ? ` · ${provider.latencyMs} ms` : ""}
        {` · ${provider.models.length} 个模型`}
      </p>
      {provider.detail ? (
        <p role="alert" className="mt-2 rounded-md border border-[var(--color-destructive)] px-2 py-1 text-xs text-[var(--color-destructive)]">
          {provider.detail}
        </p>
      ) : null}
      <div className="mt-3">
        {provider.models.length === 0 ? (
          <p className="text-xs text-[var(--color-muted-foreground)]">
            {provider.health === "unreachable"
              ? "上游不可达：地址/凭据或网络问题，恢复后自动续探。"
              : "没有已发布模型（检查节点配置的 models_allowlist）。"}
          </p>
        ) : (
          provider.models.map((model) => <ModelRow key={model.id} model={model} />)
        )}
      </div>
    </section>
  );
}

export function ModelsView({ bench }: { bench: Workbench }) {
  return (
    <div className="grid gap-4">
      {!bench.node ? (
        <p className="text-sm text-[var(--color-muted-foreground)]">目录中没有节点。</p>
      ) : bench.llmProviders.length === 0 ? (
        <p className="rounded-md border border-[var(--color-border)] px-3 py-2 text-sm text-[var(--color-muted-foreground)]">
          节点还没有 llm 提供者：在节点配置 [llm] 段并装载 node-llm 应用包后，这里会显示它发布的模型服务。
        </p>
      ) : (
        <>
          <p className="text-xs text-[var(--color-muted-foreground)]">
            模型经节点适配上游服务（OpenAI 兼容面）；hub 网关把获准的中继暴露为 /v1 接口。健康与列表每 10 秒刷新。
          </p>
          <div className="grid gap-4 md:grid-cols-2">
            {bench.llmProviders.map((provider) => (
              <ProviderCard key={provider.id} provider={provider} />
            ))}
          </div>
        </>
      )}
    </div>
  );
}
