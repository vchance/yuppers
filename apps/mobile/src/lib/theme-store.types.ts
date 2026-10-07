/** Where this device keeps its appearance choice: `light`, `dark`, or nothing for the system's. */
export interface ThemeStore {
  read(): string | null;
  write(value: string | null): void;
}
