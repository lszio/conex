import { BinaryReader, BinaryWriter } from "@bufbuild/protobuf/wire";
import { Limits, Plane } from "./common";
export declare const protobufPackage = "conex";
/**
 * 能力端点自描述。仅建模可公布的安装清单字段；
 * 源码路径、进程命令、凭据后端与允许地址属于管理员安装配置，不进入本消息。
 */
export interface Manifest {
    manifestVersion?: number | undefined;
    id?: string | undefined;
    endpointId?: string | undefined;
    kind?: string | undefined;
    display?: string | undefined;
    plane?: Plane | undefined;
    transport?: string | undefined;
    provides?: string[] | undefined;
    requires?: string[] | undefined;
    profileIds?: string[] | undefined;
    limits?: Limits | undefined;
}
export declare const Manifest: MessageFns<Manifest>;
type Builtin = Date | Function | Uint8Array | string | number | boolean | undefined;
export type DeepPartial<T> = T extends Builtin ? T : T extends globalThis.Array<infer U> ? globalThis.Array<DeepPartial<U>> : T extends ReadonlyArray<infer U> ? ReadonlyArray<DeepPartial<U>> : T extends {} ? {
    [K in keyof T]?: DeepPartial<T[K]>;
} : Partial<T>;
type KeysOfUnion<T> = T extends T ? keyof T : never;
export type Exact<P, I extends P> = P extends Builtin ? P : P & {
    [K in keyof P]: Exact<P[K], I[K]>;
} & {
    [K in Exclude<keyof I, KeysOfUnion<P>>]: never;
};
export interface MessageFns<T> {
    encode(message: T, writer?: BinaryWriter): BinaryWriter;
    decode(input: BinaryReader | Uint8Array, length?: number): T;
    fromJSON(object: any): T;
    toJSON(message: T): unknown;
    create<I extends Exact<DeepPartial<T>, I>>(base?: I): T;
    fromPartial<I extends Exact<DeepPartial<T>, I>>(object: I): T;
}
export {};
