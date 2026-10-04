import { useState } from "react";
import { Menu, type MenuEntry } from "@capia/ui-kit";

export interface MenuState {
  x: number;
  y: number;
  label: string;
  entries: MenuEntry[];
  onSelect: (id: string) => void;
}

/** Hook para menus de contexto: `open(...)` mostra, `node` é o elemento a renderizar. */
export function useContextMenu() {
  const [menu, setMenu] = useState<MenuState | null>(null);
  return {
    open: (m: MenuState) => {
      setMenu(m);
    },
    close: () => {
      setMenu(null);
    },
    node: menu ? (
      <Menu
        x={menu.x}
        y={menu.y}
        label={menu.label}
        entries={menu.entries}
        onSelect={menu.onSelect}
        onClose={() => {
          setMenu(null);
        }}
      />
    ) : null,
  };
}
