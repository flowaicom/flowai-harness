/**
 * Workspace API functions.
 *
 * CRUD operations for workspace management.
 *
 * @module api/workspaces
 */

import type { Result } from "~/lib/domain/result";
import { isOk } from "~/lib/domain/result";
import type {
  CreateWorkspaceRequest,
  ProvisionWorkspaceRequest,
  UpdateWorkspaceRequest,
  Workspace,
  WorkspaceId,
} from "~/lib/domain/workspace";
import type { ApiError } from "./client";
import { del, get, post, put } from "./client";
import { WorkspaceListSchema, WorkspaceSchema } from "./schemas";
import { validateBoundary } from "./validation";

// ============================================================================
// Workspace Endpoints
// ============================================================================

/**
 * List all workspaces for the current tenant.
 */
export async function listWorkspaces(): Promise<Result<Workspace[], ApiError>> {
  const result = await get<Workspace[]>("/workspaces");
  if (!isOk(result)) return result;
  return validateBoundary(WorkspaceListSchema, result.value, "listWorkspaces");
}

/**
 * Get a single workspace by ID.
 */
export async function getWorkspace(id: WorkspaceId): Promise<Result<Workspace, ApiError>> {
  const result = await get<Workspace>(`/workspaces/${id}`);
  if (!isOk(result)) return result;
  return validateBoundary(WorkspaceSchema, result.value, "getWorkspace");
}

/**
 * Create a new workspace.
 */
export async function createWorkspace(
  request: CreateWorkspaceRequest
): Promise<Result<Workspace, ApiError>> {
  const result = await post<Workspace>("/workspaces", request);
  if (!isOk(result)) return result;
  return validateBoundary(WorkspaceSchema, result.value, "createWorkspace");
}

/**
 * Update a workspace.
 */
export async function updateWorkspace(
  id: WorkspaceId,
  request: UpdateWorkspaceRequest
): Promise<Result<Workspace, ApiError>> {
  const result = await put<Workspace>(`/workspaces/${id}`, request);
  if (!isOk(result)) return result;
  return validateBoundary(WorkspaceSchema, result.value, "updateWorkspace");
}

/**
 * Provision a workspace with 4 databases from a single base PostgreSQL URL.
 */
export async function provisionWorkspaces(
  request: ProvisionWorkspaceRequest
): Promise<Result<Workspace, ApiError>> {
  const result = await post<Workspace>("/workspaces/provision", request);
  if (!isOk(result)) return result;
  return validateBoundary(WorkspaceSchema, result.value, "provisionWorkspaces");
}

/**
 * Delete a workspace.
 */
export async function deleteWorkspace(
  id: WorkspaceId
): Promise<Result<{ deleted: boolean }, ApiError>> {
  return del<{ deleted: boolean }>(`/workspaces/${id}`);
}
