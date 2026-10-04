// What this page is, in three claims. A visitor lands on a bare list and has no
// idea what they are looking at, so the model is stated up front rather than
// left to the protocol docs.

import { Network, ShieldCheck, Zap } from "lucide-react";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

const CLAIMS = [
  {
    icon: Network,
    title: "浏览器只连一处",
    body: "所有访问都经过同一个中心 Host。同源 HTTPS/WSS，不触碰任何内网地址、凭据或文件路径。",
  },
  {
    icon: ShieldCheck,
    title: "名字只是显示",
    body: "名称与分组由你自己填写，未经认证，也不会带来任何权限。重名会自动加数字后缀以示区分。",
  },
  {
    icon: Zap,
    title: "hello 是真往返",
    body: "点一次按钮，消息会经 Host 推送给对方的页面，对方自动应答，Host 测出往返毫秒数。",
  },
] as const;

export function Intro() {
  return (
    <Card>
      <CardHeader>
        <CardTitle>这是什么</CardTitle>
      </CardHeader>
      <CardContent className="grid gap-4 sm:grid-cols-3">
        {CLAIMS.map(({ icon: Icon, title, body }) => (
          <div key={title} className="grid gap-1.5">
            <Icon className="size-4 text-[var(--color-primary)]" aria-hidden />
            <h3 className="text-sm font-semibold">{title}</h3>
            <p className="text-sm text-[var(--color-muted-foreground)]">{body}</p>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}
