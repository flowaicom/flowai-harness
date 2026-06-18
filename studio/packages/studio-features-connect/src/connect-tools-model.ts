export const CONNECT_TOOLS_SECTION_TITLE = "Catalog";

export type ConnectToolDescriptionBlock =
  | {
      readonly kind: "paragraph";
      readonly text: string;
    }
  | {
      readonly kind: "heading";
      readonly text: string;
    }
  | {
      readonly kind: "list";
      readonly items: readonly string[];
    };

export function getConnectToolsSection<TTool>(tools: readonly TTool[]): {
  readonly title: string;
  readonly tools: readonly TTool[];
} {
  return {
    title: CONNECT_TOOLS_SECTION_TITLE,
    tools,
  };
}

export function getConnectToolDescriptionBlocks(
  description: string
): readonly ConnectToolDescriptionBlock[] {
  const blocks: ConnectToolDescriptionBlock[] = [];
  let paragraphLines: string[] = [];
  let listItems: string[] = [];

  const flushParagraph = () => {
    if (paragraphLines.length === 0) return;
    blocks.push({
      kind: "paragraph",
      text: paragraphLines.join(" ").replace(/\s+/g, " ").trim(),
    });
    paragraphLines = [];
  };

  const flushList = () => {
    if (listItems.length === 0) return;
    blocks.push({
      kind: "list",
      items: listItems,
    });
    listItems = [];
  };

  for (const rawLine of description.split(/\r?\n/)) {
    const line = rawLine.trim();

    if (line.length === 0) {
      flushParagraph();
      flushList();
      continue;
    }

    const listItem = line.match(/^[-*]\s+(.+)$/);
    if (listItem) {
      flushParagraph();
      listItems.push(listItem[1].trim());
      continue;
    }

    flushList();
    if (isToolDescriptionHeading(line)) {
      flushParagraph();
      blocks.push({
        kind: "heading",
        text: line,
      });
      continue;
    }

    paragraphLines.push(line);
  }

  flushParagraph();
  flushList();

  return blocks;
}

function isToolDescriptionHeading(line: string): boolean {
  return /^[A-Z][A-Za-z0-9 /_-]{0,48}:$/.test(line);
}
