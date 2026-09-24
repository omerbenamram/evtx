import { z } from "zod";
import { evtxFileInfoSchema } from "./types";

export type ParserCommand = { type: "open"; file: File } | { type: "chunk"; index: number };
export type ParserRequest = ParserCommand & { id: number };

export const parserResponseSchema = z.discriminatedUnion("type", [
  z.object({
    id: z.number().int(),
    type: z.literal("open"),
    info: evtxFileInfoSchema,
  }),
  z.object({
    id: z.number().int(),
    type: z.literal("chunk"),
    buffer: z.instanceof(ArrayBuffer),
    rows: z.number().int().nonnegative(),
    errors: z.array(z.string()),
  }),
  z.object({ id: z.number().int(), type: z.literal("chunk-error"), error: z.string() }),
  z.object({ id: z.number().int(), type: z.literal("error"), error: z.string() }),
]);
export type ParserResponse = z.infer<typeof parserResponseSchema>;
export class EvtxChunkError extends Error {}
