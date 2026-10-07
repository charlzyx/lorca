import { Stack, useRouter } from "expo-router";
import { useState } from "react";
import { Alert, ScrollView, Text, View } from "react-native";
import { request } from "../modules/lorca-core";
import type { AttentionPreferences, AttentionSource } from "../src/core/attention";
import { useStore } from "../src/core/store";
import { t, useLanguage } from "../src/i18n";
import { Row, Section, ToggleRow } from "../src/ui/forms";
import { CloseToolbar } from "../src/ui/navigation";
import { usePalette } from "../src/ui/theme";

export default function AttentionScreen() {
  useLanguage();
  const router = useRouter();
  const p = usePalette();
  const attention = useStore((s) => s.attention);
  const bots = useStore((s) => s.bots);
  const chats = useStore((s) => s.chats);
  const [busy, setBusy] = useState(false);
  const botName = (id: string) => bots.find((bot) => bot.id === id)?.name ?? t("Coordinator");
  async function perform(method: string, params: Record<string, unknown>) {
    setBusy(true);
    try { await request(method, params); }
    catch (error) { Alert.alert(t("Attention"), error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  }
  function preference(params: Partial<AttentionPreferences>) { if (!busy) void perform("attention.preferences", params); }
  function sourceTitle(source: AttentionSource) {
    const chat = chats.find((chat) => chat.id === source.chat_id);
    return chat?.title ?? (chat?.kind === "dm" ? botName(chat.bot_ids[0]) : t("Source chat"));
  }
  function openSource(source: AttentionSource) { router.dismissTo(`/chat/${source.chat_id}`); }
  return <>
    <Stack.Screen options={{ title: t("Attention") }} />
    <CloseToolbar label={t("Done")} onClose={() => router.back()} />
    <ScrollView contentInsetAdjustmentBehavior="automatic" style={{ backgroundColor: p.groupedBackground }} contentContainerStyle={{ padding: 16 }}>
      <Section title={t("Notifications")}>
        <ToggleRow title={t("Coordinator summaries")} value={attention.preferences.summaries} onValueChange={(summaries) => preference({ summaries })} />
        <ToggleRow title={t("Urgent direct alerts")} value={attention.preferences.urgent_direct} onValueChange={(urgent_direct) => preference({ urgent_direct })} />
        <Row title={t("Default coordinator")} menu={{ title: t("Default coordinator"), value: attention.preferences.default_coordinator_bot_id ? botName(attention.preferences.default_coordinator_bot_id) : t("Chat owner"), choices: [
          { title: t("Chat owner"), selected: !attention.preferences.default_coordinator_bot_id, onPress: () => preference({ default_coordinator_bot_id: null }) },
          ...bots.map((bot) => ({ title: bot.name, selected: attention.preferences.default_coordinator_bot_id === bot.id, onPress: () => preference({ default_coordinator_bot_id: bot.id }) })),
        ] }} />
      </Section>
      {attention.briefs.map((brief) => <Section key={brief.coordinator_bot_id} title={botName(brief.coordinator_bot_id)}>
        <View style={{ padding: 16, gap: 8 }}>
          {brief.decisions.map((decision, index) => <Text key={`decision-${index}`} selectable style={{ color: p.label }}>{t("Decision: {text}", { text: decision })}</Text>)}
          {brief.changes.map((change, index) => <Text key={`change-${index}`} selectable style={{ color: p.label }}>{t("Changed: {text}", { text: change })}</Text>)}
          <Text selectable style={{ color: p.label }}>{t("Next: {text}", { text: brief.next_action })}</Text>
        </View>
        <Row title={t("Open brief")} onPress={() => openSource({ chat_id: brief.chat_id, message_id: brief.message_id })} />
      </Section>)}
      {!attention.items.length && <Section><Row title={t("Nothing needs attention")} /></Section>}
      {attention.items.map((item) => <Section key={item.id} title={[item.urgent ? t("Urgent") : ({ review: t("Pending review"), blocker: t("Blocker"), commitment: t("Commitment"), change: t("Important change") })[item.category], botName(item.coordinator_bot_id)].join(" · ")}>
        <View style={{ padding: 16, gap: 8 }}>
          <Text selectable style={{ color: p.label, fontWeight: "600" }}>{item.title}</Text>
          <Text selectable style={{ color: p.label }}>{item.summary}</Text>
          <Text selectable style={{ color: p.label }}>{t("Next: {text}", { text: item.next_action })}</Text>
        </View>
        {item.sources.map((source, index) => <Row key={index} title={sourceTitle(source)} subtitle={source.review_id ?? source.task_id} chevron onPress={() => openSource(source)} />)}
        <Row title={t("Mark resolved")} onPress={busy ? undefined : () => void perform("attention.resolve", { id: item.id, expected_revision: item.revision })} />
      </Section>)}
    </ScrollView>
  </>;
}
