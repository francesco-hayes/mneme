import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { ModelInfo } from "../lib/api";
import type { DownloadStatus } from "../lib/models";
import { DownloadProgress } from "./DownloadProgress";
import { ModelList, ModelRow } from "./ModelRow";

// These two are also what First run shows, so they are tested by themselves
// here; the Settings tests cover them at work.

const SMALL: ModelInfo = {
  id: "small",
  name: "Small",
  sizeBytes: 487_601_967,
  note: "Balanced speed and accuracy.",
  englishOnly: false,
  installed: false,
};
const SMALL_EN: ModelInfo = {
  ...SMALL,
  id: "small.en",
  name: "Small (English)",
  sizeBytes: 487_614_201,
  note: "Balanced speed and accuracy. English only.",
  englishOnly: true,
};

const status = (
  phase: DownloadStatus["phase"],
  receivedBytes = 0,
  error: string | null = null,
): DownloadStatus => ({
  phase,
  receivedBytes,
  totalBytes: SMALL.sizeBytes,
  error,
});

function renderRow(props: Partial<Parameters<typeof ModelRow>[0]> = {}) {
  const handlers = {
    onDownload: vi.fn(),
    onCancelDownload: vi.fn(),
    onDismissError: vi.fn(),
  };
  render(
    <ModelList aria-label="Models">
      <ModelRow model={SMALL} {...handlers} {...props} />
    </ModelList>,
  );
  return { ...handlers, row: within(screen.getByRole("listitem")) };
}

test("a model that is not installed: name, note, size and Download", () => {
  const { row, onDownload } = renderRow();

  expect(screen.getByRole("listitem", { name: "Small" })).toBeInTheDocument();
  expect(row.getByText("Balanced speed and accuracy.")).toBeVisible();
  expect(row.getByText("488 MB")).toBeVisible();
  expect(row.queryByText("English only")).toBeNull();
  expect(row.queryByRole("progressbar")).toBeNull();

  fireEvent.click(row.getByRole("button", { name: "Download Small" }));
  expect(onDownload).toHaveBeenCalledTimes(1);
});

test("an English-only model is tagged, and its note does not repeat it", () => {
  const { row } = renderRow({ model: SMALL_EN });

  expect(row.getByText("English only")).toBeVisible();
  expect(row.getByText("Balanced speed and accuracy.")).toBeVisible();
});

test("a leading control and a badge are shown with the name", () => {
  const { row } = renderRow({
    leading: <input type="radio" aria-label="Pick Small" />,
    badge: "Recommended",
  });

  expect(row.getByRole("radio", { name: "Pick Small" })).toBeInTheDocument();
  expect(row.getByText("Recommended")).toBeVisible();
});

test("downloading: the bar, received of total, and Cancel in place of Download", () => {
  const { row, onCancelDownload } = renderRow({
    download: status("downloading", 121_900_492),
  });

  const bar = row.getByRole("progressbar", { name: "Downloading Small" });
  expect(bar).toHaveAttribute("value", "121900492");
  expect(bar).toHaveAttribute("max", "487601967");
  expect(row.getByText("122 MB of 488 MB")).toBeVisible();
  // The total is in that line, so the size is not shown a second time.
  expect(row.queryByText("488 MB")).toBeNull();
  expect(row.queryByRole("button", { name: /^Download/ })).toBeNull();

  fireEvent.click(
    row.getByRole("button", { name: "Cancel downloading Small" }),
  );
  expect(onCancelDownload).toHaveBeenCalledTimes(1);
});

test("starting: no value on the bar yet, and nothing to cancel", () => {
  const starting = renderRow({ download: status("starting") });
  expect(starting.row.getByText("Starting…")).toBeVisible();
  expect(starting.row.getByRole("progressbar")).not.toHaveAttribute("value");
  expect(
    starting.row.getByRole("button", { name: "Cancel downloading Small" }),
  ).toBeDisabled();
});

test("verifying keeps Cancel; cancelling turns it off", () => {
  const { row } = renderRow({
    download: status("verifying", SMALL.sizeBytes),
  });
  expect(row.getByText("Verifying…")).toBeVisible();
  expect(
    row.getByRole("button", { name: "Cancel downloading Small" }),
  ).toBeEnabled();
});

test("cancelling: the bar stays, grayed by its phase, and Cancel is off", () => {
  const { row } = renderRow({ download: status("cancelling", 121_900_492) });

  expect(row.getByText("Cancelling…")).toBeVisible();
  const bar = row.getByRole("progressbar");
  expect(bar).toHaveAttribute("value", "121900492");
  expect(bar.parentElement).toHaveAttribute("data-phase", "cancelling");
  expect(
    row.getByRole("button", { name: "Cancel downloading Small" }),
  ).toBeDisabled();
});

test("a failed download: the reason, Retry and Dismiss", () => {
  const { row, onDownload, onDismissError } = renderRow({
    download: status("error", 0, "The disk is full."),
  });

  expect(row.getByRole("alert")).toHaveTextContent(
    "Could not download Small: The disk is full.",
  );
  expect(row.queryByRole("progressbar")).toBeNull();
  expect(row.getByText("488 MB")).toBeVisible();

  fireEvent.click(row.getByRole("button", { name: "Retry downloading Small" }));
  expect(onDownload).toHaveBeenCalledTimes(1);
  fireEvent.click(
    row.getByRole("button", { name: "Dismiss the error for Small" }),
  );
  expect(onDismissError).toHaveBeenCalledTimes(1);
});

test("a cancel that could not be sent is said under the running download", () => {
  const { row } = renderRow({
    download: status("downloading", 100, "Could not cancel: No answer."),
  });

  expect(row.getByRole("alert")).toHaveTextContent(
    "Could not cancel: No answer.",
  );
  expect(row.getByRole("progressbar")).toBeInTheDocument();
});

test("an installed model says Installed, or shows the controls it is given", () => {
  const installed = { ...SMALL, installed: true };
  const first = render(
    <ModelList aria-label="Models">
      <ModelRow model={installed} onDownload={() => {}} />
    </ModelList>,
  );
  expect(screen.getByText("Installed")).toBeVisible();
  expect(screen.queryByRole("button")).toBeNull();
  first.unmount();

  render(
    <ModelList aria-label="Models">
      <ModelRow
        model={installed}
        installedActions={<button type="button">Delete</button>}
      >
        <p>Under the row</p>
      </ModelRow>
    </ModelList>,
  );
  expect(screen.queryByText("Installed")).toBeNull();
  expect(screen.getByRole("button", { name: "Delete" })).toBeInTheDocument();
  expect(screen.getByText("Under the row")).toBeVisible();
});

test("without handlers a row is read-only", () => {
  render(
    <ModelList aria-label="Models">
      <ModelRow model={SMALL} />
    </ModelList>,
  );
  expect(screen.queryByRole("button")).toBeNull();
});

test("DownloadProgress stands alone, and sets no inline style", () => {
  const { container, rerender } = render(
    <DownloadProgress
      name="Small"
      status={status("downloading", 243_800_990)}
    />,
  );

  expect(
    screen.getByRole("progressbar", { name: "Downloading Small" }),
  ).toHaveAttribute("value", "243800990");
  expect(screen.getByText("244 MB of 488 MB")).toBeVisible();
  expect(container.querySelector("[style]")).toBeNull();

  // More than the total never overfills the bar.
  rerender(
    <DownloadProgress
      name="Small"
      status={status("downloading", SMALL.sizeBytes + 5)}
    />,
  );
  expect(screen.getByRole("progressbar")).toHaveAttribute(
    "value",
    String(SMALL.sizeBytes),
  );

  // A failure is the caller's to show.
  rerender(
    <DownloadProgress name="Small" status={status("error", 0, "No.")} />,
  );
  expect(container).toBeEmptyDOMElement();
});
