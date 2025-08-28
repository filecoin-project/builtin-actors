// SPDX-License-Identifier: MIT
pragma solidity 0.8.25;


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
    uint64 constant SECTOR_CONTENT_CHANGED = 86399155;
    
    /**
     * @dev Get the count of notifications for a specific sector
     */
    function getNotificationCount(uint64 sector) public view returns (uint256) {
        return sectorNotificationIndices[sector].length;
    }
    
    /**
     * @dev Get all notification indices for a sector
     */
    function getSectorNotifications(uint64 sector) public view returns (uint256[] memory) {
        return sectorNotificationIndices[sector];
    }
    
    /**
     * @dev Get a specific notification by index
     */
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
    
    /**
     * @dev Handle incoming Filecoin method calls
     * This is the main entry point for receiving notifications from the miner actor
     */
    function handle_filecoin_method(uint64 method, uint64, bytes memory params) public returns (bytes memory) {
        // Check if this is a sector content changed notification
        if (method == SECTOR_CONTENT_CHANGED) {
            return processSectorContentChanged(params);
        }
        
        // For other methods, just revert
       revert("Invalid method");
    }
    
    /**
     * @dev Process sector content changed notification
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
     */
    function processSectorContentChanged(bytes memory params) internal returns (bytes memory) {

        (uint nSectors, uint byteIdx) = readFixedArray(params, 0);
        for (uint i = 0; i < nSectors; i++) {

            /* We now need to parse a tuple of 3 cbor objects:
                (sector, minimum_commitment_epoch, added_pieces) */
            uint checkTupleLen;
            (checkTupleLen, byteIdx) = readFixedArray(params, byteIdx);
            require(checkTupleLen == 3, "Invalid params outer");

            uint64 sector;
            (sector, byteIdx) = readUInt64(params, byteIdx);

            int64 minimumCommitmentEpoch;
            (minimumCommitmentEpoch, byteIdx) = readInt64(params, byteIdx);

            uint256 pieceCnt;
            (pieceCnt, byteIdx) = readFixedArray(params, byteIdx); 

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
            }
        }
        /* Hack: just return CBOR null == `0xF6`
          This deserializes to SectorContentChangedReturn [[bool]] but will fail validation.
          To call this without failing commitment message must specify require_success == false
        */
        return hex"81f6";
          
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
}

