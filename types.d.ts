interface Window {
  __TAURI__: {
    window: {
      getCurrentWindow(): {
        onCloseRequested(
          handler: (event: { preventDefault(): void }) => Promise<void>,
        ): Promise<unknown>;
        minimize(): Promise<void>;
        toggleMaximize(): Promise<void>;
        close(): Promise<void>;
        destroy(): Promise<void>;
      };
    };
    event: {
      listen(
        name: string,
        handler: (event: { payload: any }) => void,
      ): Promise<unknown>;
    };
    core: {
      invoke(command: string, arguments: Record<string, unknown>): Promise<any>;
    };
  };
}
