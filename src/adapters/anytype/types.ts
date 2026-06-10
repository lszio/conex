// Anytype API 类型定义

export interface AnytypeObject {
  id: string;
  name: string;
  type: string;
  description?: string;
  done?: boolean;
  deadline?: string;
  priority?: number;
  tags?: string[];
  spaceId?: string;
  lastModifiedDate: string;
  createdDate: string;
  [key: string]: unknown;
}

export interface AnytypeSpace {
  id: string;
  name: string;
  description?: string;
  icon?: string;
}

export interface AnytypeListResponse<T> {
  objects: T[];
  total?: number;
  offset?: number;
  limit?: number;
}

export interface AnytypeObjectResponse {
  object: AnytypeObject;
}

export interface AnytypeErrorResponse {
  error: {
    code: string;
    message: string;
  };
}