// SPDX-License-Identifier: MIT
pragma solidity 0.8.25;

/// @title FilecoinCBOR
/// @notice Shared CBOR parsing/writing utilities and actor call helpers for Filecoin EVM contracts.
/// All functions are `internal` (inlined at compile time), so no separate deployment is needed.
library FilecoinCBOR {
    // FVM precompiles
    address constant CALL_ACTOR_ID = 0xfe00000000000000000000000000000000000005;

    // FVM flags and codecs
    uint64 constant READ_ONLY_FLAG = 0x00000001;
    uint64 constant DEFAULT_FLAG = 0x00000000;
    uint64 constant CBOR_CODEC = 0x51;
    uint64 constant DAG_CBOR_CODEC = 0x71;
    uint64 constant NONE_CODEC = 0x00;

    // CBOR major types
    uint8 constant MajUnsignedInt = 0;
    uint8 constant MajSignedInt = 1;
    uint8 constant MajByteString = 2;
    uint8 constant MajTextString = 3;
    uint8 constant MajArray = 4;
    uint8 constant MajMap = 5;
    uint8 constant MajTag = 6;
    uint8 constant MajOther = 7;

    uint8 constant True_Type = 21;
    uint8 constant False_Type = 20;

    uint8 constant TagTypeBigNum = 2;
    uint8 constant TagTypeNegativeBigNum = 3;

    /* *** Actor call helpers *** */

    function callById(
        uint64 target,
        uint256 method_num,
        uint64 codec,
        bytes memory raw_request,
        uint256 value,
        bool static_call
    ) internal returns (int256, bytes memory) {
        (bool success, bytes memory data) = address(CALL_ACTOR_ID).delegatecall(
            abi.encode(uint64(method_num), value, static_call ? READ_ONLY_FLAG : DEFAULT_FLAG, codec, raw_request, target)
        );
        require(success, "delegatecall failed");
        return readRespData(data);
    }

    function readRespData(bytes memory raw_response) internal pure returns (int256, bytes memory) {
        (int256 exit, uint64 return_codec, bytes memory return_value) =
            abi.decode(raw_response, (int256, uint64, bytes));

        if (return_codec == NONE_CODEC) {
            require(return_value.length == 0, "invalid response length");
        } else if (return_codec == CBOR_CODEC || return_codec == DAG_CBOR_CODEC) {
            require(return_value.length > 0, "invalid response length");
        } else {
            revert("invalid codec");
        }

        return (exit, return_value);
    }

    /* *** CBOR parsing *** */

    /// @notice Parse cbor header for major type and extra info.
    function parseCborHeader(bytes memory cbor, uint byteIndex) internal pure returns (uint8, uint64, uint) {
        uint8 first = sliceUInt8(cbor, byteIndex);
        byteIndex += 1;
        uint8 maj = (first & 0xe0) >> 5;
        uint8 low = first & 0x1f;
        require(low < 28, "cannot handle headers with extra > 27");

        if (low < 24) {
            return (maj, low, byteIndex);
        }
        if (low == 24) {
            uint8 next = sliceUInt8(cbor, byteIndex);
            byteIndex += 1;
            require(next >= 24, "invalid cbor");
            return (maj, next, byteIndex);
        }
        if (low == 25) {
            uint16 extra16 = sliceUInt16(cbor, byteIndex);
            byteIndex += 2;
            return (maj, extra16, byteIndex);
        }
        if (low == 26) {
            uint32 extra32 = sliceUInt32(cbor, byteIndex);
            byteIndex += 4;
            return (maj, extra32, byteIndex);
        }
        require(low == 27, "ExpectedLowValue27");
        uint64 extra64 = sliceUInt64(cbor, byteIndex);
        byteIndex += 8;
        return (maj, extra64, byteIndex);
    }

    /// @notice Read a fixed-length CBOR array header.
    function readFixedArray(bytes memory cborData, uint byteIdx) internal pure returns (uint, uint) {
        uint8 maj;
        uint len;
        (maj, len, byteIdx) = parseCborHeader(cborData, byteIdx);
        require(maj == MajArray, "invalid maj (expected MajArray)");
        return (len, byteIdx);
    }

    /// @notice Read a uint64 value.
    function readUInt64(bytes memory cborData, uint byteIdx) internal pure returns (uint64, uint) {
        uint8 maj;
        uint value;
        (maj, value, byteIdx) = parseCborHeader(cborData, byteIdx);
        require(maj == MajUnsignedInt, "invalid maj (expected MajUnsignedInt)");
        return (uint64(value), byteIdx);
    }

    /// @notice Read an int64 value.
    function readInt64(bytes memory cborData, uint byteIdx) internal pure returns (int64, uint) {
        uint8 maj;
        uint value;
        (maj, value, byteIdx) = parseCborHeader(cborData, byteIdx);
        require(maj == MajSignedInt || maj == MajUnsignedInt, "invalid maj (expected MajSignedInt or MajUnsignedInt)");
        return (int64(uint64(value)), byteIdx);
    }

    /// @notice Read an arbitrary byte string, handling CID tags.
    function readBytes(bytes memory cborData, uint byteIdx) internal pure returns (bytes memory, uint) {
        uint8 maj;
        uint len;
        (maj, len, byteIdx) = parseCborHeader(cborData, byteIdx);
        require(maj == MajTag || maj == MajByteString, "invalid maj (expected MajTag or MajByteString)");

        if (maj == MajTag) {
            (maj, len, byteIdx) = parseCborHeader(cborData, byteIdx);
            if (!(maj == MajByteString)) {
                revert("expected MajByteString");
            }
        }

        uint max_len = byteIdx + len;
        bytes memory slice = new bytes(len);
        uint slice_index = 0;
        for (uint256 i = byteIdx; i < max_len; i++) {
            slice[slice_index] = cborData[i];
            slice_index++;
        }

        return (slice, byteIdx + len);
    }

    /// @notice Read a boolean value.
    function readBool(bytes memory cborData, uint byteIdx) internal pure returns (bool, uint) {
        uint8 maj;
        uint value;
        (maj, value, byteIdx) = parseCborHeader(cborData, byteIdx);
        require(maj == MajOther, "expected MajOther for bool");
        require(value == True_Type || value == False_Type, "invalid bool value");
        return (value == True_Type, byteIdx);
    }

    /* *** Byte slicing helpers *** */

    function sliceUInt8(bytes memory bs, uint start) private pure returns (uint8) {
        require(bs.length >= start + 1, "slicing out of range");
        return uint8(bs[start]);
    }

    function sliceUInt16(bytes memory bs, uint start) private pure returns (uint16) {
        require(bs.length >= start + 2, "slicing out of range");
        bytes2 x;
        assembly {
            x := mload(add(bs, add(0x20, start)))
        }
        return uint16(x);
    }

    function sliceUInt32(bytes memory bs, uint start) private pure returns (uint32) {
        require(bs.length >= start + 4, "slicing out of range");
        bytes4 x;
        assembly {
            x := mload(add(bs, add(0x20, start)))
        }
        return uint32(x);
    }

    function sliceUInt64(bytes memory bs, uint start) private pure returns (uint64) {
        require(bs.length >= start + 8, "slicing out of range");
        bytes8 x;
        assembly {
            x := mload(add(bs, add(0x20, start)))
        }
        return uint64(x);
    }

    /* *** CBOR writing *** */

    struct Buffer {
        bytes buf;
        uint capacity;
    }

    struct CBORBuffer {
        Buffer buf;
    }

    function createCBOR(uint256 capacity) internal pure returns(CBORBuffer memory cbor) {
        initBuffer(cbor.buf, capacity);
        return cbor;
    }

    function getCBORData(CBORBuffer memory buf) internal pure returns(bytes memory) {
        return buf.buf.buf;
    }

    function startFixedArray(CBORBuffer memory buf, uint64 length) internal pure {
        writeFixedNumeric(buf, MajArray, length);
    }

    function writeUInt64(CBORBuffer memory buf, uint64 value) internal pure {
        writeFixedNumeric(buf, MajUnsignedInt, value);
    }

    function writeBool(CBORBuffer memory buf, bool val) internal pure {
        appendUint8(buf.buf, uint8((MajOther << 5) | (val ? True_Type : False_Type)));
    }

    function writeTextString(CBORBuffer memory buf, string memory val) internal pure {
        bytes memory valBytes = bytes(val);
        writeFixedNumeric(buf, MajTextString, uint64(valBytes.length));
        appendBytes(buf.buf, valBytes);
    }

    function writeByteString(CBORBuffer memory buf, bytes memory val) internal pure {
        writeFixedNumeric(buf, MajByteString, uint64(val.length));
        appendBytes(buf.buf, val);
    }

    /* *** Internal buffer helpers *** */

    function writeFixedNumeric(CBORBuffer memory buf, uint8 major, uint64 val) private pure {
        if (val <= 23) {
            appendUint8(buf.buf, uint8((major << 5) | val));
        } else if (val <= 0xFF) {
            appendUint8(buf.buf, uint8((major << 5) | 24));
            appendInt(buf.buf, val, 1);
        } else if (val <= 0xFFFF) {
            appendUint8(buf.buf, uint8((major << 5) | 25));
            appendInt(buf.buf, val, 2);
        } else if (val <= 0xFFFFFFFF) {
            appendUint8(buf.buf, uint8((major << 5) | 26));
            appendInt(buf.buf, val, 4);
        } else {
            appendUint8(buf.buf, uint8((major << 5) | 27));
            appendInt(buf.buf, val, 8);
        }
    }

    function initBuffer(Buffer memory buf, uint capacity) private pure {
        if (capacity % 32 != 0) {
            capacity += 32 - (capacity % 32);
        }
        buf.capacity = capacity;
        assembly {
            let ptr := mload(0x40)
            mstore(buf, ptr)
            mstore(ptr, 0)
            let fpm := add(32, add(ptr, capacity))
            if lt(fpm, ptr) {
                revert(0, 0)
            }
            mstore(0x40, fpm)
        }
    }

    function appendUint8(Buffer memory buf, uint8 val) private pure {
        uint off = buf.buf.length;
        uint offPlusOne = off + 1;
        if (off >= buf.capacity) {
            resizeBuffer(buf, offPlusOne * 2);
        }

        assembly {
            let bufptr := mload(buf)
            let dest := add(add(bufptr, off), 32)
            mstore8(dest, val)
            if gt(offPlusOne, mload(bufptr)) {
                mstore(bufptr, offPlusOne)
            }
        }
    }

    function appendInt(Buffer memory buf, uint val, uint len) private pure {
        uint off = buf.buf.length;
        uint newCapacity = len + off;
        if (newCapacity > buf.capacity) {
            resizeBuffer(buf, newCapacity * 2);
        }

        uint mask = (256 ** len) - 1;
        assembly {
            let bufptr := mload(buf)
            let dest := add(bufptr, newCapacity)
            mstore(dest, or(and(mload(dest), not(mask)), val))
            if gt(newCapacity, mload(bufptr)) {
                mstore(bufptr, newCapacity)
            }
        }
    }

    function resizeBuffer(Buffer memory buf, uint capacity) private pure {
        bytes memory oldbuf = buf.buf;
        initBuffer(buf, capacity);
        appendBytes(buf, oldbuf);
    }

    function appendBytes(Buffer memory buf, bytes memory val) private pure {
        uint len = val.length;
        uint off = buf.buf.length;
        uint newCapacity = off + len;
        if (newCapacity > buf.capacity) {
            resizeBuffer(buf, newCapacity * 2);
        }

        uint dest;
        uint src;
        assembly {
            let bufptr := mload(buf)
            let buflen := mload(bufptr)
            dest := add(add(bufptr, 32), off)
            if gt(newCapacity, buflen) {
                mstore(bufptr, newCapacity)
            }
            src := add(val, 32)
        }

        for (; len >= 32; len -= 32) {
            assembly {
                mstore(dest, mload(src))
            }
            dest += 32;
            src += 32;
        }

        if (len > 0) {
            uint mask = (256 ** (32 - len)) - 1;
            assembly {
                let srcpart := and(mload(src), not(mask))
                let destpart := and(mload(dest), mask)
                mstore(dest, or(destpart, srcpart))
            }
        }
    }
}
