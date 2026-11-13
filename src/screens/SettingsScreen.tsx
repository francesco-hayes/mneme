import { useEffect } from "react";
import { Group, GroupRow, InlineError, Screen, Section } from "../components";
import type { Settings } from "../lib/api";
import { cx } from "../lib/cx";
import { useModelLibrary } from "../state/modelLibrary";
import { ModelsSection } from "./settings/ModelsSection";
import { ShortcutRecorder } from "./settings/ShortcutRecorder";
import { useSettingSaver } from "./settings/useSettingSaver";
import styles from "./SettingsScreen.module.css";

/**
 * Settings: the transcription models and which one is the default, and the
 * global shortcut. Everything is saved as it is changed. The models, settings
 * and downloads live in the model library, so the screen shows a download
 * exactly where it stands each time it is opened.
 */
export function SettingsScreen() {
  const library = useModelLibrary();
  const { load } = library;
  // Read again each time Settings is shown; what was read before stays on
  // screen meanwhile.
  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Screen title="Settings" settings={false}>
      {library.status === "error" ? (
        <InlineError
          className={styles.loadError}
          message={`Could not load the settings: ${library.loadError ?? ""}`}
          onRetry={() => void load()}
        />
      ) : library.settings === null ? (
        <p className={cx(styles.loading, "appears-late")}>Loading settings…</p>
      ) : (
        <SettingsContent settings={library.settings} />
      )}
    </Screen>
  );
}

function SettingsContent({ settings }: { settings: Settings }) {
  const defaultModel = useSettingSaver(
    "defaultModelId",
    settings.defaultModelId,
  );
  const shortcut = useSettingSaver("globalShortcut", settings.globalShortcut);

  return (
    <>
      <ModelsSection defaultModel={defaultModel} />

      <Section title="Shortcut">
        <Group>
          <GroupRow
            label="Start or stop recording"
            description="A key combination that works from any app."
          >
            <ShortcutRecorder
              value={shortcut.value}
              saving={shortcut.saving}
              error={shortcut.error}
              onChange={(value) => void shortcut.save(value)}
            />
          </GroupRow>
        </Group>
      </Section>
    </>
  );
}
