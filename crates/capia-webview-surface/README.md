# capia-webview-surface

Superfície do preview P2 (ADR-069/074): região de memória compartilhada com o WebView2 (`CreateSharedBuffer` + `PostSharedBufferToScript`). Contrato do buffer: cabeçalho de 64 B (`"CAPF"`, `seq`, `width`, `height`, LE) + RGBA8 reto. **`unsafe` só aqui** (`win.rs` COM, `FrameRegion` com limites verificados e testado em Linux); fora do Windows só existe `FrameRegion`. A UI confirma `sharedbufferreceived` antes de usar e cai para o IPC binário em qualquer falha.
