import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";
import App from "./App";
import { mockApi, mockScenarios } from "./lib/api/mock";

const heading = (name: string) => screen.findByRole("heading", { name });

describe("first-run gate", () => {
  test("opens on Home when a model is installed", async () => {
    render(<App />);

    expect(await heading("Meetings")).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Welcome to Mneme" }),
    ).not.toBeInTheDocument();
  });

  test("opens on First run when no model is installed", async () => {
    mockApi.mock.reset(mockScenarios["first-run"]);
    render(<App />);

    expect(await heading("Welcome to Mneme")).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Meetings" }),
    ).not.toBeInTheDocument();
    // First run is not a place to wander off from.
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Settings" })).toBeNull();
  });

  test("opens on First run when models exist but none is installed", async () => {
    mockApi.mock.setInstalledModels([]);
    render(<App />);

    expect(await heading("Welcome to Mneme")).toBeInTheDocument();
  });

  test("shows why startup failed, and Retry tries again", async () => {
    const user = userEvent.setup();
    mockApi.mock.failNext("listModels", "The models folder is missing.");
    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Mneme could not start: The models folder is missing.",
    );
    await user.click(screen.getByRole("button", { name: "Retry" }));

    expect(await heading("Meetings")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("navigation", () => {
  test("Home has no Back button", async () => {
    render(<App />);

    await heading("Meetings");
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
  });

  test("goes to Settings and back to Home", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole("button", { name: "Settings" }));
    expect(await heading("Settings")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Meetings" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(await heading("Meetings")).toBeInTheDocument();
  });

  test("opens a meeting by id and returns to Home", async () => {
    const user = userEvent.setup();
    render(<App />);

    const list = await screen.findByRole("list", { name: "Meetings" });
    await user.click(within(list).getByRole("button", { name: /^Standup/ }));
    expect(await heading("Standup")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(await heading("Meetings")).toBeInTheDocument();
  });

  test("Back retraces the way: meeting, Settings, meeting, Home", async () => {
    const user = userEvent.setup();
    render(<App />);

    const list = await screen.findByRole("list", { name: "Meetings" });
    await user.click(
      within(list).getByRole("button", { name: /^Design review/ }),
    );
    await heading("Design review");
    await user.click(screen.getByRole("button", { name: "Settings" }));
    await heading("Settings");

    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(await heading("Design review")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(await heading("Meetings")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
  });

  test("a meeting that cannot be opened shows the error in place", async () => {
    const user = userEvent.setup();
    mockApi.mock.failNext("getMeeting", "The folder was moved.");
    render(<App />);

    const list = await screen.findByRole("list", { name: "Meetings" });
    await user.click(within(list).getByRole("button", { name: /^Standup/ }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not open this meeting: The folder was moved.",
    );
    expect(screen.getByRole("button", { name: "Back" })).toBeInTheDocument();
  });
});
