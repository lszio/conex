// Apple Reminders 相关类型定义

export interface ReminderData {
  id?: string;
  title: string;
  notes?: string;
  dueDate?: Date;
  isCompleted?: boolean;
  priority?: number;        // 0=none, 1=high, 5=medium, 9=low
  alarmDate?: Date;         // remind me date
  recurrence?: string;      // JSON string for EKRecurrenceRule
  list?: string;            // 列表名称，默认 "CONEX-Anytype"
  conexId?: string;         // CONEX 追踪 ID (在备注中存储)
}

export interface ReminderList {
  name: string;
  count: number;
}

export interface AppleRemindersStatus {
  available: boolean;
  remindersRunning: boolean;
  version?: string;
  error?: string;
}