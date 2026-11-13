import { render, screen, within } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "./App";
import { mockApi } from "./lib/api/mock";

test("lists the mock meetings, newest first, and marks the interrupted one", async () => {
  render(<App />);

  const rows = await screen.findAllByRole("listitem");
  expect(rows).toHaveLength(4);
  expect(within(rows[0]).getByText("Standup")).toBeInTheDocument();
  expect(within(rows[0]).getByText("14 min")).toBeInTheDocument();
  expect(within(rows[1]).getByText("Design review")).toBeInTheDocument();
  expect(within(rows[1]).getByText("interrupted")).toBeInTheDocument();
  expect(within(rows[3]).getByText("Quarterly planning")).toBeInTheDocument();
});

test("shows an empty state when there are no meetings", async () => {
  mockApi.mock.reset({ meetings: [] });
  render(<App />);

  expect(await screen.findByText("No meetings yet.")).toBeInTheDocument();
});

test("shows the error when the list cannot be loaded", async () => {
  mockApi.mock.failNext("listMeetings", "The meetings folder is missing.");
  render(<App />);

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Could not load meetings: The meetings folder is missing.",
  );
});
