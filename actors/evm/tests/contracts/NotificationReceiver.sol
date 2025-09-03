// SPDX-License-Identifier: MIT
pragma solidity 0.8.25;

// ========================================================
// NOTE: If using this contract as an example, consider depending on utilities
// available in https://github.com/filecoin-project/filecoin-solidity instead of
// copying reusable utilities from here.
// ========================================================

contract NotificationReceiver {
    // State variables to track received notifications
    struct SectorNotification {
        uint64 sector;
        int64 minimumCommitmentEpoch;
        bytes dataCid;
        uint64 pieceSize;
        bytes payload;
    }
    
    SectorNotification[] public notifications;
    mapping(uint64 => uint256[]) public sectorNotificationIndices;
    
    // Counter for total notifications received
    uint256 public totalNotifications;
    
    // Flag to test different response behaviors
    bool public shouldRejectNotifications = false;
    
    // Method selector for handle_filecoin_method
    bytes4 constant NATIVE_METHOD_SELECTOR = 0x868e10c4;
    
    // Sector content changed method number
    uint64 constant SECTOR_CONTENT_CHANGED = 2034386435;
    
    // Get the count of notifications for a specific sector
    function getNotificationCount(uint64 sector) public view returns (uint256) {
        return sectorNotificationIndices[sector].length;
    }
    
    // Get all notification indices for a sector
    function getSectorNotifications(uint64 sector) public view returns (uint256[] memory) {
        return sectorNotificationIndices[sector];
    }
    
    // Get a specific notification by index
    function getNotification(uint256 index) public view returns (
        uint64 sector,
        int64 minimumCommitmentEpoch,
        bytes memory dataCid,
        uint64 pieceSize,
        bytes memory payload
    ) {
        require(index < notifications.length, "Invalid notification index");
        SectorNotification memory notif = notifications[index];
        return (
            notif.sector,
            notif.minimumCommitmentEpoch,
            notif.dataCid,
            notif.pieceSize,
            notif.payload
        );
    }
    
    // Handle incoming Filecoin method calls
    // This is the main entry point for receiving notifications from the miner actor
    function handle_filecoin_method(uint64 method, uint64 inCodec, bytes memory params) public returns (uint64, uint64,bytes memory) {
        // 0x51 is IPLD CBOR codec 
        require(inCodec == 0x51, "Invalid codec");
        // Check if this is a sector content changed notification
        if (method == SECTOR_CONTENT_CHANGED) {
            bytes memory ret = processSectorContentChanged(params);
            uint64 codec = 0x51;
            return (0, codec, ret);
        }
        
        // For other methods, just revert
       revert("Invalid method");
    }
    
    /**
     * Process sector content changed notification
     * Expected params structure (CBOR encoded):
     * {
     *   sectors: [{
     *     sector: uint64,
     *     minimum_commitment_epoch: int64,
     *     added: [{
     *       data: Cid,
     *       size: uint64,
     *       payload: bytes
     *     }]
     *   }]
     * }
     *
     * All notifications are accepted so CBOR true returned for every piece of every notified sector
     */
    function processSectorContentChanged(bytes memory params) internal returns (bytes memory) {
        require(isMinerActor(msg.sender), "Only miner actor can call this function");

        uint checkTupleLen;
        uint byteIdx = 0;

        // We don't need to parse the SectorContentChangedParams as a tuple because
        // the type is encoded as serde transparent.  So just parse the sectors array directly
        uint nSectors;
        (nSectors, byteIdx) = readFixedArray(params, byteIdx);
        require(nSectors > 0, "Invalid non positive sectors field");

        CBORBuffer memory ret_acc;
        {
            // Setup return value ret_acc
            // ret_acc accumulates return cbor array 
            ret_acc = createCBOR(64);
            // No SectorContentChangedReturn outer tuple as it is serde transparent
            startFixedArray(ret_acc, uint64(nSectors)); // sectors: Vec<SectorReturn>
        }
        for (uint i = 0; i < nSectors; i++) {

            /* We now need to parse a tuple of 3 cbor objects:
                (sector, minimum_commitment_epoch, added_pieces) */
            (checkTupleLen, byteIdx) = readFixedArray(params, byteIdx);
            require(checkTupleLen == 3, "Invalid SectorChanges tuple");


            uint64 sector;
            (sector, byteIdx) = readUInt64(params, byteIdx);

            int64 minimumCommitmentEpoch;
            (minimumCommitmentEpoch, byteIdx) = readInt64(params, byteIdx);

            uint256 pieceCnt;
            (pieceCnt, byteIdx) = readFixedArray(params, byteIdx); 

            {
                // No SectorReturn outer tuple as it is serde transparent
                startFixedArray(ret_acc, uint64(pieceCnt)); // added: Vec<PieceReturn>
            }

            for (uint j = 0; j < pieceCnt; j++) {
                /* We now need to parse a tuple of 3 cbor objects: 
                    (data, size, payload)
                */
                (checkTupleLen, byteIdx) = readFixedArray(params, byteIdx);
                require(checkTupleLen == 3, "Invalid params inner");

                bytes memory dataCid;
                (dataCid, byteIdx) = readBytes(params, byteIdx);

                uint64 pieceSize;
                (pieceSize, byteIdx) = readUInt64(params, byteIdx);

                bytes memory payload;
                (payload, byteIdx) = readBytes(params, byteIdx); 

                // Store the notification
                uint256 notificationIndex = notifications.length;
                notifications.push(SectorNotification({
                    sector: sector,
                    minimumCommitmentEpoch: minimumCommitmentEpoch,
                    dataCid: dataCid,
                    pieceSize: pieceSize,
                    payload: payload
                }));
                
                sectorNotificationIndices[sector].push(notificationIndex);
                totalNotifications++;
                {
                    // No PieceReturn outer tuple as it is serde transparent
                    writeBool(ret_acc, true); // accepted (set all to true)
                }
            }
        }

        return getCBORData(ret_acc);
    }

    /* Filecoin internal call helpers to enable isMiner check */

    // FVM specific precompiles
    address constant RESOLVE_ADDRESS_PRECOMPILE_ADDR = 0xFE00000000000000000000000000000000000001;
    address constant CALL_ACTOR_ID = 0xfe00000000000000000000000000000000000005;
    
    // FVM system flags 
    uint64 constant READ_ONLY_FLAG = 0x00000001;
    uint64 constant DEFAULT_FLAG = 0x00000000;    
    uint64 constant DAG_CBOR_CODEC = 0x71;
    uint64 constant CBOR_CODEC = 0x51;
    uint64 constant NONE_CODEC = 0x00;


    // Power actor constants 
    uint64 constant MINER_RAW_POWER_METHOD_NUMBER = 3753401894;
    uint64 constant POWER_ACTOR_ID = 4;

    // msg.sender to actor id conversion
    address constant U64_MASK = 0xFffFfFffffFfFFffffFFFffF0000000000000000;
    address constant ZERO_ID_ADDRESS = 0xfF00000000000000000000000000000000000000;
    address constant MAX_U64 = 0x000000000000000000000000fFFFFFffFFFFfffF;


    function isMinerActor(address caller) internal returns (bool) {
        (bool isNative, uint64 minerID) = isIDAddress(caller);
        require(isNative, "caller is not an ID addr");
        CBORBuffer memory buf = createCBOR(8);
        writeUInt64(buf, minerID);
        bytes memory rawRequest = getCBORData(buf);
        (int256 exit,) = callById(POWER_ACTOR_ID, MINER_RAW_POWER_METHOD_NUMBER, CBOR_CODEC, rawRequest, 0, false);
        // If the call succeeds, the address is a registered miner
        return exit == 0;

    }

    function isIDAddress(address _a) internal pure returns (bool isID, uint64 id) {
        /// @solidity memory-safe-assembly
        assembly {
            // Zeroes out the last 8 bytes of _a
            let a_mask := and(_a, U64_MASK)

            // If the result is equal to the ZERO_ID_ADDRESS,
            // _a is an ID address.
            if eq(a_mask, ZERO_ID_ADDRESS) {
                isID := true
                id := and(_a, MAX_U64)
            }
        }
    }

    // Stripped down version of callByID to query power actor in our use case 
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
        if (!success) {
            revert("fail to call actor");
        }

        return readRespData(data);
    }

    function readRespData(bytes memory raw_response) internal pure returns (int256, bytes memory) {
        (int256 exit, uint64 return_codec, bytes memory return_value) = abi.decode(raw_response, (int256, uint64, bytes));

        if (return_codec == NONE_CODEC) {
            if (return_value.length != 0) {
                revert("invalid response length");
            }
        } else if (return_codec == CBOR_CODEC || return_codec == DAG_CBOR_CODEC) {
            if (return_value.length == 0) {
                revert("invalid response length");
            }
        } else {
            revert("invalid codec");
        }

        return (exit, return_value);
    }

   


    /* *** CBOR parsing *** */

    uint8 constant MajUnsignedInt = 0;
    uint8 constant MajSignedInt = 1;
    uint8 constant MajByteString = 2;
    uint8 constant MajTextString = 3;
    uint8 constant MajArray = 4;
    uint8 constant MajMap = 5;
    uint8 constant MajTag = 6;
    uint8 constant MajOther = 7;

    uint8 constant TagTypeBigNum = 2;
    uint8 constant TagTypeNegativeBigNum = 3;

    uint8 constant True_Type = 21;
    uint8 constant False_Type = 20;

    /// @notice attempt to read the length of a fixed array
    /// @param cborData cbor encoded bytes to parse from
    /// @param byteIdx current position to read on the cbor encoded bytes
    /// @return length of the fixed array decoded from input bytes and the byte index after moving past the value
    function readFixedArray(bytes memory cborData, uint byteIdx) internal pure returns (uint, uint) {
        uint8 maj;
        uint len;

        (maj, len, byteIdx) = parseCborHeader(cborData, byteIdx);
        require(maj == MajArray, "invalid maj (expected MajArray)");

        return (len, byteIdx);
    }

    /// @notice attempt to read an arbitrary byte string value
    /// @param cborData cbor encoded bytes to parse from
    /// @param byteIdx current position to read on the cbor encoded bytes
    /// @return arbitrary byte string decoded from input bytes and the byte index after moving past the value
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

    /// @notice attempt to read a uint64 value
    /// @param cborData cbor encoded bytes to parse from
    /// @param byteIdx current position to read on the cbor encoded bytes
    /// @return an uint64 decoded from input bytes and the byte index after moving past the value
    function readUInt64(bytes memory cborData, uint byteIdx) internal pure returns (uint64, uint) {
        uint8 maj;
        uint value;

        (maj, value, byteIdx) = parseCborHeader(cborData, byteIdx);
        require(maj == MajUnsignedInt, "invalid maj (expected MajUnsignedInt)");

        return (uint64(value), byteIdx);
    }

    /// @notice attempt to read a int64 value
    /// @param cborData cbor encoded bytes to parse from
    /// @param byteIdx current position to read on the cbor encoded bytes
    /// @return an int64 decoded from input bytes and the byte index after moving past the value
    function readInt64(bytes memory cborData, uint byteIdx) internal pure returns (int64, uint) {
        uint8 maj;
        uint value;

        (maj, value, byteIdx) = parseCborHeader(cborData, byteIdx);
        require(maj == MajSignedInt || maj == MajUnsignedInt, "invalid maj (expected MajSignedInt or MajUnsignedInt)");

        return (int64(uint64(value)), byteIdx);
    }

    /// @notice Parse cbor header for major type and extra info.
    /// @param cbor cbor encoded bytes to parse from
    /// @param byteIndex current position to read on the cbor encoded bytes
    /// @return major type, extra info and the byte index after moving past header bytes
    function parseCborHeader(bytes memory cbor, uint byteIndex) internal pure returns (uint8, uint64, uint) {
        uint8 first = sliceUInt8(cbor, byteIndex);
        byteIndex += 1;
        uint8 maj = (first & 0xe0) >> 5;
        uint8 low = first & 0x1f;
        // We don't handle CBOR headers with extra > 27, i.e. no indefinite lengths
        require(low < 28, "cannot handle headers with extra > 27");

        // extra is lower bits
        if (low < 24) {
            return (maj, low, byteIndex);
        }

        // extra in next byte
        if (low == 24) {
            uint8 next = sliceUInt8(cbor, byteIndex);
            byteIndex += 1;
            require(next >= 24, "invalid cbor"); // otherwise this is invalid cbor
            return (maj, next, byteIndex);
        }

        // extra in next 2 bytes
        if (low == 25) {
            uint16 extra16 = sliceUInt16(cbor, byteIndex);
            byteIndex += 2;
            return (maj, extra16, byteIndex);
        }

        // extra in next 4 bytes
        if (low == 26) {
            uint32 extra32 = sliceUInt32(cbor, byteIndex);
            byteIndex += 4;
            return (maj, extra32, byteIndex);
        }

        // extra in next 8 bytes
        if (!(low == 27)) {
            revert("ExpectedLowValue27");
        }
        uint64 extra64 = sliceUInt64(cbor, byteIndex);
        byteIndex += 8;
        return (maj, extra64, byteIndex);
    } 

     /// @notice slice uint8 from bytes starting at a given index
    /// @param bs bytes to slice from
    /// @param start current position to slice from bytes
    /// @return uint8 sliced from bytes
    function sliceUInt8(bytes memory bs, uint start) internal pure returns (uint8) {
        require(bs.length >= start + 1, "slicing out of range");
        return uint8(bs[start]);
    }

    /// @notice slice uint16 from bytes starting at a given index
    /// @param bs bytes to slice from
    /// @param start current position to slice from bytes
    /// @return uint16 sliced from bytes
    function sliceUInt16(bytes memory bs, uint start) internal pure returns (uint16) {
        require(bs.length >= start + 2, "slicing out of range");
        bytes2 x;
        assembly {
            x := mload(add(bs, add(0x20, start)))
        }
        return uint16(x);
    }

    /// @notice slice uint32 from bytes starting at a given index
    /// @param bs bytes to slice from
    /// @param start current position to slice from bytes
    /// @return uint32 sliced from bytes
    function sliceUInt32(bytes memory bs, uint start) internal pure returns (uint32) {
        require(bs.length >= start + 4, "slicing out of range");
        bytes4 x;
        assembly {
            x := mload(add(bs, add(0x20, start)))
        }
        return uint32(x);
    }

    /// @notice slice uint64 from bytes starting at a given index
    /// @param bs bytes to slice from
    /// @param start current position to slice from bytes
    /// @return uint64 sliced from bytes
    function sliceUInt64(bytes memory bs, uint start) internal pure returns (uint64) {
        require(bs.length >= start + 8, "slicing out of range");
        bytes8 x;
        assembly {
            x := mload(add(bs, add(0x20, start)))
        }
        return uint64(x);
    }

    /* *** CBOR writing *** */
    // === MINIMAL CBOR ENCODING FOR SectorContentChangedReturn ===

    // Buffer struct
    struct Buffer {
        bytes buf;
        uint capacity;
    }

    struct CBORBuffer {
        Buffer buf;
    }

    // Create a new CBOR buffer with given capacity
    function createCBOR(uint256 capacity) internal pure returns(CBORBuffer memory cbor) {
        initBuffer(cbor.buf, capacity);
        return cbor;
    }

    // Get the encoded bytes from the buffer
    function getCBORData(CBORBuffer memory buf) internal pure returns(bytes memory) {
        return buf.buf.buf;
    }

    // Start a fixed-length array
    function startFixedArray(CBORBuffer memory buf, uint64 length) internal pure {
        writeFixedNumeric(buf, MajArray, length);
    }

    // Write a boolean value
    function writeBool(CBORBuffer memory buf, bool val) internal pure {
        appendUint8(buf.buf, uint8((MajOther << 5) | (val ? True_Type : False_Type)));
    }

    // Write a Uint64 value
    function writeUInt64(CBORBuffer memory buf, uint64 value) internal pure {
        writeFixedNumeric(buf, MajUnsignedInt, value);
    }

    // === INTERNAL HELPER FUNCTIONS ===

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
        
        // Copy word-length chunks
        for (; len >= 32; len -= 32) {
            assembly {
                mstore(dest, mload(src))
            }
            dest += 32;
            src += 32;
        }
        
        // Copy remaining bytes
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

