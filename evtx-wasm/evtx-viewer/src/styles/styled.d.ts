import "styled-components";
import { theme } from "./theme";

type AppTheme = typeof theme;

declare module "styled-components" {
  // eslint-disable-next-line @typescript-eslint/no-empty-interface
  export interface DefaultTheme extends AppTheme {
    /**
     * NOTE: This dummy property exists solely to avoid TypeScript's
     * "empty interface" restriction when `no-empty-interface` is enabled.
     * It has no practical effect at runtime.
     */
    readonly __brand?: "DefaultTheme";
  }
}
