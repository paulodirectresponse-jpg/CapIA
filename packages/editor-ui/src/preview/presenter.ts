import type { FrameData, FrameSink } from "./scheduler";

const VS = `attribute vec2 p;varying vec2 uv;void main(){uv=vec2((p.x+1.)*.5,1.-(p.y+1.)*.5);gl_Position=vec4(p,0.,1.);}`;
const FS = `precision mediump float;varying vec2 uv;uniform sampler2D t;void main(){gl_FragColor=texture2D(t,uv);}`;

/**
 * Apresentador do preview (ADR-069 / P2): textura RGBA8 → canvas WebGL. Sem WebGL (ambiente
 * sem GPU, jsdom) cai para `putImageData` no canvas 2D — a imagem é a mesma, só muda o caminho.
 */
export class CanvasPresenter implements FrameSink {
  mode: FrameSink["mode"] = "none";
  private gl: WebGLRenderingContext | null = null;
  private ctx2d: CanvasRenderingContext2D | null = null;
  private tex: WebGLTexture | null = null;
  private size = { w: 0, h: 0 };

  constructor(private readonly canvas: HTMLCanvasElement) {
    this.initGl();
    if (!this.gl) {
      this.ctx2d = canvas.getContext("2d");
      this.mode = this.ctx2d ? "2d" : "none";
    }
  }

  private initGl(): void {
    let gl: WebGLRenderingContext | null = null;
    try {
      gl = this.canvas.getContext("webgl", {
        alpha: false,
        antialias: false,
        preserveDrawingBuffer: true,
      });
    } catch {
      gl = null;
    }
    if (!gl) return;
    const sh = (type: number, src: string) => {
      const s = gl.createShader(type);
      if (!s) return null;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null;
    };
    const vs = sh(gl.VERTEX_SHADER, VS);
    const fs = sh(gl.FRAGMENT_SHADER, FS);
    const prog = gl.createProgram();
    if (!vs || !fs) return;
    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) return;
    gl.useProgram(prog);
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, "p");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    this.tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    this.gl = gl;
    this.mode = "webgl";
  }

  present(frame: FrameData): void {
    const { width, height, rgba } = frame;
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }
    if (this.gl && this.tex) {
      const gl = this.gl;
      gl.viewport(0, 0, width, height);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      if (this.size.w !== width || this.size.h !== height) {
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
        this.size = { w: width, h: height };
      } else {
        gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
      }
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    } else if (this.ctx2d) {
      const data = new ImageData(
        new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength),
        width,
        height,
      );
      this.ctx2d.putImageData(data, 0, 0);
    }
  }

  dispose(): void {
    if (this.gl) {
      if (this.tex) this.gl.deleteTexture(this.tex);
      this.gl.getExtension("WEBGL_lose_context")?.loseContext();
    }
    this.gl = null;
    this.ctx2d = null;
    this.tex = null;
    this.mode = "none";
  }
}
