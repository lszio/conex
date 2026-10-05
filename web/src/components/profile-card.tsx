// Self profile: name, group and visibility. The name field shows what the host
// actually assigned, not what was typed, because duplicates are suffixed there.

import { useEffect, useState } from "react";

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
            <Label htmlFor="profile-group">组</Label>
            <Input
              id="profile-group"
              maxLength={24}
              placeholder="用于自己分类"
              value={group}
              onChange={(event) => setGroup(event.target.value)}
            />
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
                group: group.trim(),
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
