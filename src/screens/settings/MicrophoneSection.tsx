import { useId } from "react";
import {
  ChevronUpDownIcon,
  Group,
  GroupRow,
  InlineError,
  Section,
} from "../../components";
import { api, type Microphone } from "../../lib/api";
import { useLoad } from "../../lib/useLoad";
import styles from "./MicrophoneSection.module.css";
import type { SettingSaver } from "./useSettingSaver";

const loadMicrophones = () => api.listMicrophones();

/** The option that stands for "no choice": the system default input. */
const SYSTEM_DEFAULT = "";

export interface MicrophoneSectionProps {
  /** The microphone setting: a device id, or null for the system default. */
  microphone: SettingSaver<string | null>;
}

/**
 * Which input device records your side of a meeting: the system default
 * (the default choice), or one device by name. The list is read again each
 * time the menu is about to open, so a microphone plugged in meanwhile is
 * there. A chosen device that is not connected stays chosen: meetings record
 * the system default until it is back.
 */
export function MicrophoneSection({ microphone }: MicrophoneSectionProps) {
  const labelId = useId();
  const devices = useLoad(loadMicrophones);
  const list: readonly Microphone[] =
    devices.status === "ready" ? devices.data : [];
  const systemDefault = list.find((device) => device.isDefault);
  const chosen = microphone.value;
  const chosenMissing =
    chosen !== null &&
    devices.status === "ready" &&
    !list.some((device) => device.id === chosen);

  return (
    <Section title="Microphone">
      <Group>
        <GroupRow
          label="Record my side with"
          labelId={labelId}
          description="A change applies from the next meeting."
          below={
            <>
              {chosenMissing && (
                <p className={styles.note}>
                  That microphone is not connected. Meetings record the system
                  default until it is plugged in.
                </p>
              )}
              {devices.status === "error" && (
                <InlineError
                  className={styles.message}
                  message={`Could not list the microphones: ${devices.message}`}
                  onRetry={devices.reload}
                />
              )}
              {microphone.error !== null && (
                <InlineError
                  className={styles.message}
                  message={`Could not change the microphone: ${microphone.error}`}
                />
              )}
            </>
          }
        >
          <span className={styles.field}>
            <select
              className={styles.select}
              aria-labelledby={labelId}
              value={chosen ?? SYSTEM_DEFAULT}
              // Plugged in or out since the list was read: read it again
              // before the menu opens.
              onMouseDown={devices.reload}
              onKeyDown={(event) => {
                if (event.key === " " || event.key.startsWith("Arrow")) {
                  devices.reload();
                }
              }}
              onChange={(event) => {
                const value = event.target.value;
                void microphone.save(value === SYSTEM_DEFAULT ? null : value);
              }}
            >
              <option value={SYSTEM_DEFAULT}>
                {systemDefault !== undefined
                  ? `System default (${systemDefault.name})`
                  : "System default"}
              </option>
              {list.map((device) => (
                <option key={device.id} value={device.id}>
                  {device.name}
                </option>
              ))}
              {chosen !== null && !list.some((d) => d.id === chosen) && (
                <option value={chosen}>Unplugged microphone</option>
              )}
            </select>
            <ChevronUpDownIcon />
          </span>
        </GroupRow>
      </Group>
    </Section>
  );
}
