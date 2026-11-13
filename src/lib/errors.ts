/** The text to show for something a rejected API call threw. */
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
