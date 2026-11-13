interface ImportMetaEnv {
  /**
   * Forces which API implementation the UI uses: `mock` or `tauri`. Unset, the
   * UI uses Tauri inside the app and the mock everywhere else.
   */
  readonly VITE_MNEME_API?: string;
}
