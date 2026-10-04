//! The guided request walkthrough: the same chain the console performs,
//! stepped for a visitor who has not connected yet. Method names and stage
//! order mirror what `ConexWsClient` and the Host actually do, so the
//! narrative cannot drift from the implementation.

interface Step {
  /** Short label shown on the left. */
  title: string;
  /** What the browser is doing at this step. */
  detail: string;
  /** The concrete call, shown verbatim. */
  wire: string;
  /** Where the data actually comes from. */
  actor: "浏览器" | "中心 Host" | "远端 Agent";
}

const STEPS: Step[] = [
  {
    title: "取得会话",
    detail: "同源请求。公开部署下自动签发只读匿名会话；私有部署提交管理员签发的凭据。凭据不进 URL、不进存储。",
    wire: "GET /web/session",
    actor: "浏览器",
  },
  {
    title: "换一次性票据",
    detail: "带会话 cookie 与 CSRF nonce 换取 30 秒有效、只消费一次的连接票据。",
    wire: "POST /tickets  →  { ticket }",
    actor: "浏览器",
  },
  {
    title: "升级连接",
    detail: "票据随 query 原子消费，服务端重新核对 Origin 与目标 Host，随后才接受业务帧。",
    wire: "GET /wss?ticket=…   (101 Switching Protocols)",
    actor: "浏览器",
  },
  {
    title: "协商能力",
    detail: "固定顺序握手。服务端能力来自实际角色与可达注册，不回显客户端自报的能力。",
    wire: "conex/hello → conex/ready",
    actor: "中心 Host",
  },
  {
    title: "投影目录",
    detail: "只返回当前主体获准的端点：显示名、地域、连接状态、可用方法与授权范围。绝不返回绝对路径、上游地址或凭据。",
    wire: "endpoint/list",
    actor: "中心 Host",
  },
  {
    title: "按需路由",
    detail: "Host 用 MethodContract 逐次校验租户与资源策略，再把请求转发给持有该端点的 Agent；Agent 侧再校验一次根目录范围。",
    wire: "source/list · source/read · source/search",
    actor: "远端 Agent",
  },
  {
    title: "同源取回字节",
    detail: "内容经授权探针后按切片流式返回，支持 Range 与 ETag。未知二进制不做 UTF-8 转换，下载字节与原文件一致。",
    wire: "GET /content?endpointId=…&resourceId=…",
    actor: "中心 Host",
  },
];

const ACTOR_CLASS: Record<Step["actor"], string> = {
  "浏览器": "actor-browser",
  "中心 Host": "actor-host",
  "远端 Agent": "actor-agent",
};

/** Milliseconds between steps; enough to read, short enough to replay. */
const STEP_MS = 700;

export function renderWalkthrough(host: HTMLElement, replay?: HTMLButtonElement): void {
  let timer = 0;

  const clear = (): void => {
    window.clearInterval(timer);
    host.replaceChildren();
  };

  const build = (): HTMLElement[] => {
    const list = document.createElement("ol");
    list.className = "walkthrough";
    const nodes = STEPS.map((step) => {
      const item = document.createElement("li");
      item.className = `walk-step ${ACTOR_CLASS[step.actor]}`;
      const title = document.createElement("strong");
      title.textContent = step.title;
      const detail = document.createElement("span");
      detail.className = "walk-detail";
      detail.textContent = step.detail;
      const wire = document.createElement("code");
      wire.className = "walk-wire";
      wire.textContent = step.wire;
      const actor = document.createElement("span");
      actor.className = "walk-actor";
      actor.textContent = step.actor;
      item.append(title, actor, detail, wire);
      list.append(item);
      return item;
    });
    host.append(list);
    return nodes;
  };

  const play = (nodes: HTMLElement[]): void => {
    let index = 0;
    const step = (): void => {
      if (index > 0) nodes[index - 1]?.classList.add("done");
      nodes[index]?.classList.add("active");
      index += 1;
      if (index >= nodes.length) {
        window.clearInterval(timer);
        nodes[nodes.length - 1]?.classList.add("done");
        return;
      }
      timer = window.setTimeout(step, STEP_MS);
    };
    step();
  };

  const start = (): void => {
    clear();
    play(build());
  };

  replay?.addEventListener("click", () => {
    start();
  });
  start();
}
