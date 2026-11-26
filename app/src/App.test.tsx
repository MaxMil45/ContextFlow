// The interface against the sample backend. Each test guards something that
// was fixed by looking at the real screens, so it can't quietly come back.

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { App } from "./App";

/** Open the app and pick a workspace in the sidebar by name. */
async function open(name: string) {
  const user = userEvent.setup();
  render(<App />);
  await user.click(await screen.findByText(name, { selector: ".ws-name" }));
  await screen.findByRole("heading", { level: 1, name: new RegExp(`^${name}`) });
  return user;
}

describe("Overview", () => {
  it("shows where you left off before the save controls", async () => {
    await open("Spider");
    const left = await screen.findByText("Where you left off");
    const capture = screen.getByText("Save what's open now");
    // DOCUMENT_POSITION_FOLLOWING: `capture` comes after `left`.
    expect(left.compareDocumentPosition(capture) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByText(/Fixing the retry logic/)).toBeInTheDocument();
  });

  it("shows tabs and folders next to windows", async () => {
    await open("Spider");
    expect(await screen.findByText("1 window · 7 tabs")).toBeInTheDocument();
    expect(screen.getAllByText("2 windows · 2 folders").length).toBeGreaterThan(0);
  });

  it("offers no save setting before anything is saved", async () => {
    await open("Brand new workspace");
    expect(await screen.findByRole("heading", { name: "Nothing saved yet" })).toBeInTheDocument();
    expect(screen.queryByText("When you leave this workspace")).not.toBeInTheDocument();
    expect(screen.getByText("Save what's open now")).toBeInTheDocument();
  });
});

describe("History", () => {
  it("names the version on one line and offers to delete older ones", async () => {
    const user = await open("Spider");
    await user.click(screen.getByRole("tab", { name: /History/ }));
    expect(await screen.findByRole("heading", { name: /Version 4/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete older…" })).toBeInTheDocument();
    // The same summary as the Overview, tabs and folders included.
    expect(screen.getByText("1 window · 7 tabs")).toBeInTheDocument();
  });

  it("offers no 'Delete older' with a single version", async () => {
    const user = await open("Chess");
    await user.click(screen.getByRole("tab", { name: /History/ }));
    await screen.findByRole("heading", { name: /Version 1/ });
    expect(screen.queryByRole("button", { name: "Delete older…" })).not.toBeInTheDocument();
  });
});

describe("Restore", () => {
  it("previews with 'Running' and 'Will start', never 'Skipped'", async () => {
    const user = await open("Spider");
    await user.click(screen.getByRole("button", { name: /Preview/ }));
    expect(await screen.findByText("Restore preview", { exact: false })).toBeInTheDocument();
    expect((await screen.findAllByText("Running")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("Will start").length).toBeGreaterThan(0);
    expect(screen.queryByText("Skipped")).not.toBeInTheDocument();
  });

  it("after a failure says it is finished and offers a retry by program name", async () => {
    const user = await open("Spider");
    await user.click(screen.getByRole("button", { name: /Preview/ }));
    await user.click(await screen.findByRole("button", { name: "Start restore" }));
    expect(await screen.findByText(/Restore finished/)).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Retry Spotify" })).toBeInTheDocument();
    expect(screen.getByText("its window did not appear within 30 seconds")).toBeInTheDocument();
  });
});

describe("Command palette", () => {
  it("keeps Tab on its search box instead of the page behind", async () => {
    const user = await open("Spider");
    await user.keyboard("{Control>}k{/Control}");
    const palette = await screen.findByRole("dialog", { name: "Command palette" });
    const search = within(palette).getByRole("combobox");
    expect(search).toHaveFocus();
    await user.tab();
    expect(search).toHaveFocus();
    await user.tab({ shift: true });
    expect(search).toHaveFocus();
  });
});
