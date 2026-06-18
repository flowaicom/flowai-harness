import { describe, expect, test } from "bun:test";
import { createWorkspaceScope } from "@studio/core/domain/scope";
import { renderToStaticMarkup } from "react-dom/server";
import { ConnectDiscoveryPage } from "./connect-discovery-page";

describe("Connect package page rendering", () => {
  test("renders discovery page with a fake host runtime", () => {
    const scope = createWorkspaceScope("workspace-a");
    const html = renderToStaticMarkup(
      <ConnectDiscoveryPage
        scope={scope}
        scopeKey="workspace-a"
        hasTarget={true}
        runtime={{
          async listTables() {
            return { _tag: "Ok", value: [] };
          },
          async getTableDetail() {
            return {
              _tag: "Ok",
              value: {
                schemaName: "public",
                tableName: "orders",
                rowCount: 12,
                columns: [],
                constraints: [],
                indexes: [],
              },
            };
          },
        }}
        tables={[
          {
            schemaName: "public",
            tableName: "orders",
            tableType: "BASE TABLE",
            rowCount: 12,
            columnCount: 4,
            description: null,
          },
        ]}
        tableDetail={null}
        setTables={() => {}}
        setTableDetail={() => {}}
        loadRelationships={async () => ({
          _tag: "Ok",
          value: {
            tableName: "orders",
            references: [],
            referencedBy: [],
            totalCount: 0,
          },
        })}
        emptyState={{
          title: "No data source selected",
          description: "Select a source to browse tables and columns.",
        }}
      />
    );

    expect(html).toContain("Discovery");
    expect(html).toContain("orders");
    expect(html).toContain("BASE TABLE");
    expect(html).toContain("Select a table");
    expect(html).toContain("items-start");
    expect(html).toContain("pt-10");
  });
});
