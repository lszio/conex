// Self profile: name, group and visibility. The name field shows what the host
// actually assigned, not what was typed, because duplicates are suffixed there.

import { useEffect, useState } from "react";
import { Check, Copy, Dices } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, Input, Label, Separator } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import type { CachedProfile } from "@/hooks/use-hello-page";

export interface ProfileCardProps {
  draft: CachedProfile;
  notice: string;
  onChange: (next: CachedProfile) => void;
  onSave: (next: CachedProfile) => void;
}

export function ProfileCard({ draft, notice, onChange, onSave }: ProfileCardProps) {
  // Local mirror so typing is not fought by the parent's re-render.
  const [name, setName] = useState(draft.displayName);
  const [group, setGroup] = useState(draft.group);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    setName(draft.displayName);
    setGroup(draft.group);
  }, [draft.displayName, draft.group]);

  return (
    <Card>
      <CardHeader>
        <CardTitle>我的设置</CardTitle>
      </CardHeader>
      <CardContent className="grid gap-4">
        <div className="grid gap-2 sm:grid-cols-2">
          <div className="grid gap-1.5">
            <Label htmlFor="profile-name">名称</Label>
            <Input
              id="profile-name"
              maxLength={40}
              placeholder="留空则自动分配"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </div>
          <div className="grid gap-1.5">
            <div className="flex items-center justify-between gap-2">
              <Label htmlFor="profile-group">组名</Label>
              <span className="flex items-center gap-0.5">
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7 w-7 px-0"
                  aria-label="生成随机组名"
                  title="生成随机组名"
                  onClick={() => {
                    // Crockford-ish alphabet: no ambiguous 0/O/1/I/L, so a
                    // group name read aloud or pasted half-remembered still
                    // lands in the right group.
                    const alphabet = "23456789abcdefghjkmnpqrstvwxyz";
                    const values = crypto.getRandomValues(new Uint8Array(10));
                    setGroup([...values].map((v) => alphabet[v % alphabet.length]).join(""));
                  }}
                >
                  <Dices className="size-3.5" aria-hidden />
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7 w-7 px-0"
                  aria-label={copied ? "已复制" : "复制组名"}
                  title="复制组名，发给想邀请的人"
                  onClick={() => {
                    void navigator.clipboard
                      ?.writeText(group.trim() || "default")
                      .then(() => setCopied(true))
                      .finally(() => window.setTimeout(() => setCopied(false), 1500));
                  }}
                >
                  {copied ? <Check className="size-3.5" aria-hidden /> : <Copy className="size-3.5" aria-hidden />}
                </Button>
              </span>
            </div>
            <Input
              id="profile-group"
              maxLength={24}
              placeholder="default"
              value={group}
              onChange={(event) => setGroup(event.target.value)}
            />
            <span className="text-xs text-[var(--color-muted-foreground)]">
              组名不会展示给任何人；知道组名的人才能进入同一分组并看到彼此和文件。想邀请谁，把组名发给谁。
            </span>
          </div>
        </div>

        <div className="flex items-center justify-between gap-4 rounded-md border border-[var(--color-border)] px-3 py-2.5">
          <div className="grid gap-0.5">
            <Label htmlFor="profile-visible">对其他人可见</Label>
            <span className="text-xs text-[var(--color-muted-foreground)]">
              关闭后你不出现在别人的列表里，也无法被问候
            </span>
          </div>
          <Switch
            id="profile-visible"
            checked={draft.visible}
            onCheckedChange={(checked) => onChange({ ...draft, visible: checked })}
          />
        </div>

        <Separator />

        <div className="flex flex-wrap items-center gap-3">
          <Button
            onClick={() => {
              const next = {
                displayName: name.trim(),
                group: group.trim() || "default",
                visible: draft.visible,
              };
              onChange(next);
              onSave(next);
            }}
          >
            保存
          </Button>
          {notice ? (
            <span className="text-xs text-[var(--color-muted-foreground)]" role="status">
              {notice}
            </span>
          ) : null}
        </div>
      </CardContent>
    </Card>
  );
}
