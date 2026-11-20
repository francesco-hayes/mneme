import { useId, useState } from "react";
import { Group, GroupRow, InlineError, Section } from "../../components";
import { useModelLibrary } from "../../state/modelLibrary";
import { useRecording } from "../../state/recording";
import { DownloadStrip, ModelPicker } from "./ModelPicker";
import styles from "./ModelsSection.module.css";
import type { SettingSaver } from "./useSettingSaver";

export interface ModelsSectionProps {
  /** The default model setting: its value is a model id or null. */
  defaultModel: SettingSaver<string | null>;
}

/**
 * The model manager, as one settings row: a `ModelPicker` that shows the
 * model new meetings use and opens a menu to pick another, download more or
 * delete one.
 *
 * What must be seen without opening the menu is under the row: that no model
 * is installed, a download's progress or failure, and a default that could
 * not be saved. Downloads are shown there only while the menu is closed; open,
 * the menu's own rows show them.
 */
export function ModelsSection({ defaultModel }: ModelsSectionProps) {
  const library = useModelLibrary();
  const { isActive: recordingActive } = useRecording();
  const [open, setOpen] = useState(false);
  const labelId = useId();

  const noneInstalled = !library.models.some((model) => model.installed);
  const downloading = library.models.filter(
    (model) => !model.installed && library.downloads.has(model.id),
  );

  return (
    <Section title="Transcription">
      <Group>
        <GroupRow
          label="Model for new meetings"
          labelId={labelId}
          description="Larger models are more accurate and slower."
          below={
            <>
              {noneInstalled && (
                <p className={styles.note}>
                  No model is installed. Mneme needs one to record: download one
                  from the menu.
                </p>
              )}
              {!open &&
                downloading.map((model) => {
                  const download = library.downloads.get(model.id);
                  return (
                    download !== undefined && (
                      <DownloadStrip
                        key={model.id}
                        model={model}
                        download={download}
                      />
                    )
                  );
                })}
              {defaultModel.error !== null && (
                <InlineError
                  className={styles.message}
                  message={`Could not change the default model: ${defaultModel.error}`}
                />
              )}
              {library.defaultError !== null && (
                <InlineError
                  className={styles.message}
                  message={`Could not set the default model: ${library.defaultError}`}
                  onRetry={() => void library.load()}
                />
              )}
            </>
          }
        >
          <ModelPicker
            labelId={labelId}
            defaultId={defaultModel.value}
            open={open}
            onOpenChange={setOpen}
            locked={recordingActive}
            onChoose={(id) => {
              if (id !== defaultModel.value) void defaultModel.save(id);
            }}
          />
        </GroupRow>
      </Group>
    </Section>
  );
}
