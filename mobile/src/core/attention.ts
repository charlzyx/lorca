// Wire projection from the Rust core; encrypted persistence belongs to the core.
export interface AttentionRevision { counter: number; device_id: string }
export interface AttentionSource { chat_id: string; task_id?: string; message_id?: string; review_id?: string }
export interface AttentionItem {
  id: string; category: "review" | "blocker" | "commitment" | "change"; title: string; summary: string;
  next_action: string; coordinator_bot_id: string; sources: AttentionSource[]; reporters: string[];
  urgent: boolean; revision: AttentionRevision;
}
export interface AttentionBrief {
  coordinator_bot_id: string; chat_id: string; decisions: string[]; changes: string[];
  next_action: string; item_ids: string[]; message_id: string;
}
export interface AttentionPreferences {
  summaries: boolean; urgent_direct: boolean; default_coordinator_bot_id: string | null; coordinators: Record<string, string>;
}
export interface AttentionView { items: AttentionItem[]; briefs: AttentionBrief[]; preferences: AttentionPreferences }
export function emptyAttention(): AttentionView {
  return { items: [], briefs: [], preferences: { summaries: true, urgent_direct: true, default_coordinator_bot_id: null, coordinators: {} } };
}
