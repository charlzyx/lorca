// The update's progress on Android, as a Material dialog while the release downloads and the
// package installer takes it: Install Update starts it from the offer or from Settings › Updates.
// Hide puts it away; the download goes on, and Settings' Updates row still shows how far it is.

import { AlertDialog, Column, Host, LinearProgressIndicator, Spacer, Text as ComposeText, TextButton } from "@expo/ui/jetpack-compose";
import { fillMaxWidth, height } from "@expo/ui/jetpack-compose/modifiers";
import { useEffect, useState } from "react";
import { Platform, StyleSheet } from "react-native";
import { useUpdates } from "../core/updates";
import { t } from "../i18n";
import { usePalette } from "./theme";

/// Mounted once at the root on Android.
export function UpdateProgress() {
  const phase = useUpdates((s) => s.phase);
  const progress = useUpdates((s) => s.progress);
  const latest = useUpdates((s) => s.latest?.version ?? "");
  const p = usePalette();
  const active = phase === "downloading" || phase === "installing";
  const [hidden, setHidden] = useState(false);
  // The next install shows it again.
  useEffect(() => {
    if (!active) setHidden(false);
  }, [active]);
  if (Platform.OS !== "android" || !active || hidden) return null;
  const downloading = phase === "downloading";
  return (
    <Host style={StyleSheet.absoluteFill} pointerEvents="box-none">
      <AlertDialog
        onDismissRequest={() => setHidden(true)}
        properties={{ dismissOnClickOutside: false }}
        colors={{ containerColor: p.cell as string, titleContentColor: p.label as string, textContentColor: p.secondaryLabel as string }}
      >
        <AlertDialog.Title>
          <ComposeText style={{ typography: "headlineSmall" }}>{t("Updating Lorca")}</ComposeText>
        </AlertDialog.Title>
        <AlertDialog.Text>
          <Column modifiers={[fillMaxWidth()]}>
            <ComposeText style={{ typography: "bodyMedium" }}>
              {downloading
                ? t("Downloading {latest}… {percent}%", { latest, percent: Math.round(progress * 100) })
                : t("Installing {latest}… Lorca restarts when it is done.", { latest })}
            </ComposeText>
            <Spacer modifiers={[height(16)]} />
            {/* Indeterminate while installing: the installer reports no progress. */}
            <LinearProgressIndicator progress={downloading ? progress : null} color={p.tint as string} modifiers={[fillMaxWidth()]} />
          </Column>
        </AlertDialog.Text>
        <AlertDialog.ConfirmButton>
          <TextButton onClick={() => setHidden(true)} colors={{ contentColor: p.tint as string }}>
            <ComposeText>{t("Hide")}</ComposeText>
          </TextButton>
        </AlertDialog.ConfirmButton>
      </AlertDialog>
    </Host>
  );
}
