import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Button, Dialog, IconButton, Menu, NumberField, Splitter, Tabs } from "./index";

afterEach(cleanup);

describe("Dialog", () => {
  it("focuses inside, traps Tab, closes on Escape and restores the opener's focus", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    function Host() {
      return (
        <>
          <button type="button">opener</button>
          <Dialog title="Rename" open onClose={onClose} footer={<Button>OK</Button>}>
            <input aria-label="name" />
          </Dialog>
        </>
      );
    }
    render(<Host />);
    expect(screen.getByRole("dialog", { name: "Rename" })).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByLabelText("name"));
    await user.tab();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "OK" }));
    await user.tab(); // volta ao primeiro (armadilha de foco)
    expect(document.activeElement).toBe(screen.getByLabelText("name"));
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });
});

describe("Menu", () => {
  it("navigates with the arrow keys, activates with Enter and closes with Escape", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onClose = vi.fn();
    render(
      <Menu
        label="Clip"
        x={10}
        y={10}
        entries={[
          { type: "item", id: "split", label: "Split" },
          { type: "item", id: "off", label: "Disabled", disabled: true },
          { type: "separator" },
          { type: "item", id: "del", label: "Delete", danger: true },
        ]}
        onSelect={onSelect}
        onClose={onClose}
      />,
    );
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "Split" }));
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "Delete" }));
    await user.keyboard("{Enter}");
    expect(onSelect).toHaveBeenCalledWith("del");
    expect(onClose).toHaveBeenCalled();
    onClose.mockClear();
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });
});

describe("Tabs", () => {
  it("exposes the tablist semantics and moves with the arrow keys", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <Tabs
        label="Sequences"
        active="a"
        items={[
          { id: "a", label: "A", closable: true, title: "A" },
          { id: "b", label: "B", closable: true, title: "B" },
        ]}
        onSelect={onSelect}
        onClose={vi.fn()}
      />,
    );
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.getAttribute("aria-selected"))).toEqual(["true", "false"]);
    tabs[0]?.focus();
    await user.keyboard("{ArrowRight}");
    expect(onSelect).toHaveBeenCalledWith("b");
  });
});

describe("NumberField", () => {
  it("commits once on Enter, clamps to the range and cancels on Escape", async () => {
    const user = userEvent.setup();
    const onCommit = vi.fn();
    render(
      <NumberField label="Opacity" value={0.5} min={0} max={1} step={0.1} onCommit={onCommit} />,
    );
    const input = screen.getByLabelText("Opacity");
    await user.clear(input);
    await user.type(input, "5{Enter}");
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledWith(1); // 5 fora da faixa → clamp em 1
  });

  it("does not commit when the value did not change and ArrowDown steps", async () => {
    const user = userEvent.setup();
    const onCommit = vi.fn();
    render(<NumberField label="X" value={10} min={0} max={100} step={2} onCommit={onCommit} />);
    const input = screen.getByLabelText("X");
    input.focus();
    await user.keyboard("{ArrowDown}");
    expect(onCommit).toHaveBeenCalledWith(8);
  });
});

describe("Splitter", () => {
  it("resizes with the keyboard within limits and reports the commit", async () => {
    const user = userEvent.setup();
    const onResize = vi.fn();
    const onCommit = vi.fn();
    render(
      <Splitter
        dir="col"
        label="Inspector"
        size={300}
        min={200}
        max={320}
        onResize={onResize}
        onCommit={onCommit}
      />,
    );
    screen.getByRole("separator", { name: "Inspector" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(onResize).toHaveBeenLastCalledWith(316);
    await user.keyboard("{Shift>}{ArrowRight}{/Shift}");
    expect(onResize).toHaveBeenLastCalledWith(320);
    expect(onCommit).toHaveBeenCalledTimes(2);
  });
});

describe("IconButton", () => {
  it("has an accessible name and reflects the pressed state", () => {
    render(<IconButton icon="magnet" label="Snapping" pressed />);
    const b = screen.getByRole("button", { name: "Snapping" });
    expect(b.getAttribute("aria-pressed")).toBe("true");
  });
});
