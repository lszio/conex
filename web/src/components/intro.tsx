// What conex is, stated up front.
//
// The page used to lead with "hello", which made a transport demo look like
// the product. conex is a bidirectional capability routing kernel; the hello
// scene and the file scene are two things it does, and this is the paragraph
// that says so before either of them runs.

import { Boxes, Link2, Network } from "lucide-react";

import { Badge, Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

const PROBLEMS = [
  {
    icon: Network,
    title: "中心服务要接多个异构供给方",
    body: "文件、API、数据库、远端主机各说各话。conex 用同一套 provider 契约把它们接进一个可嵌入的内核，调用者身份、策略、凭据和路由留在 host。",
  },
  {
    icon: Link2,
    title: "连接是双向的",
    body: "私有侧不会被迫开放入站端口：agent 以出站 WebSocket 反连中心，凭据留在自己这一侧，host 负责认证、授权与远端路由。",
  },
  {
    icon: Boxes,
    title: "契约先于实现",
    body: "一份 .proto 生成 Rust、TypeScript 与 JSON Schema；错误码表、内容寻址、分块与限额都是冻结契约，跨语言共享向量验证。",
  },
] as const;

export function Intro() {
  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader>
          <CardTitle>conex 是什么</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-3 text-sm leading-relaxed text-[var(--color-muted-foreground)]">
          <p>
            <strong className="text-[var(--color-foreground)]">
              conex（connect + nexus）是一个可嵌入的双向能力路由内核
            </strong>
            ，也可以作为独立进程运行。中心 host 统一做认证、授权、端点目录投影与路由；
            供给方（文件、HTTP 目录、反向连接的 agent）通过同一份 provider 契约接入。
          </p>
          <p>
            这个页面运行的就是一个真实的 conex-host 进程，不是模拟。下面的两个场景是在线
            客户端之间真实发生的事：<strong className="text-[var(--color-foreground)]">
              hello 场景
            </strong>
            测的是消息经过中心再返回的往返延迟，
            <strong className="text-[var(--color-foreground)]">文件场景</strong>
            测的是同组客户端之间的文件传递与预览。
          </p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>它解决的三件事</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-4 sm:grid-cols-3">
          {PROBLEMS.map(({ icon: Icon, title, body }) => (
            <div key={title} className="grid gap-1.5">
              <Icon className="size-4 text-[var(--color-primary)]" aria-hidden />
              <h3 className="text-sm font-semibold">{title}</h3>
              <p className="text-sm text-[var(--color-muted-foreground)]">{body}</p>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex-row items-center justify-between space-y-0">
          <CardTitle>这个演示的边界</CardTitle>
          <Badge variant="outline">只读场景 + 同组临时文件</Badge>
        </CardHeader>
        <CardContent className="grid gap-2 text-sm text-[var(--color-muted-foreground)]">
          <p>
            这里的每个客户端就是一条浏览器连接。名字与组由你自己填写，未经认证，也不会带来任何
            权限。组是隔离键：不同组的客户端互相看不见、也发不了 hello，共享文件同样只在
            组内可见。
          </p>
          <p>
            这台 Host 不提供命令执行，也不改写任何远端状态。共享的文件只存在于内存里，
            提供者断开即回收——它是能力演示，不是文件托管服务。
          </p>
          <p className="mt-1">
            同一站点的机器可读说明在{" "}
            <a className="font-medium text-[var(--color-primary)] underline underline-offset-4" href="/llms.txt">
              llms.txt
            </a>
            。
          </p>
        </CardContent>
      </Card>
    </div>
  );
}
