import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import { renderWithProviders } from "../test/render";
import {
  Button,
  EmptyState,
  GearIcon,
  IconButton,
  InlineError,
  List,
  ListRow,
  Screen,
} from ".";

test("Button is a plain button by default and passes attributes through", async () => {
  const user = userEvent.setup();
  const onClick = vi.fn();
  render(
    <>
      <Button onClick={onClick}>Download</Button>
      <Button disabled>Delete</Button>
    </>,
  );

  const download = screen.getByRole("button", { name: "Download" });
  expect(download).toHaveAttribute("type", "button");
  await user.click(download);
  expect(onClick).toHaveBeenCalledOnce();
  expect(screen.getByRole("button", { name: "Delete" })).toBeDisabled();
});

test("IconButton is named by its label", () => {
  render(
    <IconButton label="Settings">
      <GearIcon />
    </IconButton>,
  );

  const button = screen.getByRole("button", { name: "Settings" });
  expect(button).toHaveAttribute("title", "Settings");
});

test("ListRow with onActivate is one button; trailing controls stay separate", async () => {
  const user = userEvent.setup();
  const onActivate = vi.fn();
  const onDelete = vi.fn();
  render(
    <List aria-label="Models">
      <ListRow
        title="Base"
        detail="Fast. Fine for clear audio."
        meta="148 MB"
        onActivate={onActivate}
        trailing={<Button onClick={onDelete}>Delete</Button>}
      />
      <ListRow title="Small" />
    </List>,
  );

  const rows = within(
    screen.getByRole("list", { name: "Models" }),
  ).getAllByRole("listitem");
  expect(rows).toHaveLength(2);
  const open = within(rows[0]).getByRole("button", { name: /Base/ });
  expect(open).toHaveTextContent("Fast. Fine for clear audio.");
  expect(open).toHaveTextContent("148 MB");

  await user.click(within(rows[0]).getByRole("button", { name: "Delete" }));
  expect(onDelete).toHaveBeenCalledOnce();
  expect(onActivate).not.toHaveBeenCalled();
  await user.click(open);
  expect(onActivate).toHaveBeenCalledOnce();

  // Without onActivate the row is not a control.
  expect(within(rows[1]).queryByRole("button")).toBeNull();
});

test("EmptyState shows its title, description and action", () => {
  render(
    <EmptyState
      title="No meetings yet"
      description="A meeting you record shows up here."
      action={<Button>New meeting</Button>}
    />,
  );

  expect(
    screen.getByRole("heading", { name: "No meetings yet" }),
  ).toBeInTheDocument();
  expect(
    screen.getByText("A meeting you record shows up here."),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "New meeting" }),
  ).toBeInTheDocument();
});

test("InlineError is an alert, with Retry only when it can retry", async () => {
  const user = userEvent.setup();
  const onRetry = vi.fn();
  const { rerender } = render(<InlineError message="Disk full." />);

  expect(screen.getByRole("alert")).toHaveTextContent("Disk full.");
  expect(screen.queryByRole("button")).toBeNull();

  rerender(<InlineError message="Disk full." onRetry={onRetry} />);
  await user.click(screen.getByRole("button", { name: "Retry" }));
  expect(onRetry).toHaveBeenCalledOnce();
});

test("Screen puts the title, actions and Settings in the top bar", () => {
  renderWithProviders(
    <Screen title="Meetings" actions={<Button>New meeting</Button>}>
      <p>Content</p>
    </Screen>,
  );

  const topBar = screen.getByRole("banner");
  expect(
    within(topBar).getByRole("heading", { level: 1, name: "Meetings" }),
  ).toBeInTheDocument();
  expect(
    within(topBar).getByRole("button", { name: "New meeting" }),
  ).toBeInTheDocument();
  expect(
    within(topBar).getByRole("button", { name: "Settings" }),
  ).toBeInTheDocument();
  // Nothing to go back to on the first screen.
  expect(within(topBar).queryByRole("button", { name: "Back" })).toBeNull();
  expect(within(screen.getByRole("main")).getByText("Content")).toBeVisible();
});

test("Screen takes an element as its title and can drop Settings", () => {
  renderWithProviders(
    <Screen title={<input aria-label="Title" />} settings={false} back>
      <p>Content</p>
    </Screen>,
  );

  const topBar = screen.getByRole("banner");
  expect(within(topBar).getByRole("textbox", { name: "Title" })).toBeVisible();
  expect(within(topBar).queryByRole("heading")).toBeNull();
  expect(within(topBar).queryByRole("button", { name: "Settings" })).toBeNull();
  expect(within(topBar).getByRole("button", { name: "Back" })).toBeVisible();
});
