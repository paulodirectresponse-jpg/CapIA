import { describe, expect, it } from "vitest";
import { trackLabels } from "./trackNames";

const W = { video: "Vídeo", audio: "Áudio", captions: "Legendas" };

describe("trackLabels", () => {
  it("numera por tipo de baixo para cima e trata nomes de fábrica como sem nome", () => {
    const m = trackLabels(
      [
        { id: "a", kind: "visual", name: "Main", role: "main" },
        { id: "b", kind: "visual", name: "Overlay", role: "overlay" },
        { id: "c", kind: "audio", name: "Voice", role: "voice" },
        { id: "d", kind: "visual", name: "Captions", role: "captions" },
        { id: "e", kind: "audio", name: "", role: "custom" },
      ],
      W,
    );
    expect(m.get("a")).toBe("Vídeo 1");
    expect(m.get("b")).toBe("Vídeo 2");
    expect(m.get("c")).toBe("Áudio 1");
    expect(m.get("d")).toBe("Legendas");
    expect(m.get("e")).toBe("Áudio 2");
  });

  it("respeita o nome que o usuário deu", () => {
    const m = trackLabels([{ id: "a", kind: "visual", name: "B-roll do cliente" }], W);
    expect(m.get("a")).toBe("B-roll do cliente");
  });
});
