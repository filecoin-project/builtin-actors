// SPDX-License-Identifier: MIT
pragma solidity 0.8.25;

import "./FilecoinCBOR.sol";

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

    // Process sector content changed notification
    // Expected params structure (CBOR encoded):
    // {
    //   sectors: [{
    //     sector: uint64,
    //     minimum_commitment_epoch: int64,
    //     added: [{
    //       data: Cid,
    //       size: uint64,
    //       payload: bytes
    //     }]
    //   }]
    // }
    //
    // All notifications are accepted so CBOR true returned for every piece of every notified sector
    function processSectorContentChanged(bytes memory params) internal returns (bytes memory) {
        require(isMinerActor(msg.sender), "Only miner actor can call this function");

        uint checkTupleLen;
        uint byteIdx = 0;

        // We don't need to parse the SectorContentChangedParams as a tuple because
        // the type is encoded as serde transparent.  So just parse the sectors array directly
        uint nSectors;
        (nSectors, byteIdx) = FilecoinCBOR.readFixedArray(params, byteIdx);
        require(nSectors > 0, "Invalid non positive sectors field");

        FilecoinCBOR.CBORBuffer memory ret_acc;
        {
            // Setup return value ret_acc
            // ret_acc accumulates return cbor array
            ret_acc = FilecoinCBOR.createCBOR(64);
            // No SectorContentChangedReturn outer tuple as it is serde transparent
            FilecoinCBOR.startFixedArray(ret_acc, uint64(nSectors)); // sectors: Vec<SectorReturn>
        }
        for (uint i = 0; i < nSectors; i++) {

            /* We now need to parse a tuple of 3 cbor objects:
                (sector, minimum_commitment_epoch, added_pieces) */
            (checkTupleLen, byteIdx) = FilecoinCBOR.readFixedArray(params, byteIdx);
            require(checkTupleLen == 3, "Invalid SectorChanges tuple");


            uint64 sector;
            (sector, byteIdx) = FilecoinCBOR.readUInt64(params, byteIdx);

            int64 minimumCommitmentEpoch;
            (minimumCommitmentEpoch, byteIdx) = FilecoinCBOR.readInt64(params, byteIdx);

            uint256 pieceCnt;
            (pieceCnt, byteIdx) = FilecoinCBOR.readFixedArray(params, byteIdx);

            {
                // No SectorReturn outer tuple as it is serde transparent
                FilecoinCBOR.startFixedArray(ret_acc, uint64(pieceCnt)); // added: Vec<PieceReturn>
            }

            for (uint j = 0; j < pieceCnt; j++) {
                /* We now need to parse a tuple of 3 cbor objects:
                    (data, size, payload)
                */
                (checkTupleLen, byteIdx) = FilecoinCBOR.readFixedArray(params, byteIdx);
                require(checkTupleLen == 3, "Invalid params inner");

                bytes memory dataCid;
                (dataCid, byteIdx) = FilecoinCBOR.readBytes(params, byteIdx);

                uint64 pieceSize;
                (pieceSize, byteIdx) = FilecoinCBOR.readUInt64(params, byteIdx);

                bytes memory payload;
                (payload, byteIdx) = FilecoinCBOR.readBytes(params, byteIdx);

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
                    FilecoinCBOR.writeBool(ret_acc, true); // accepted (set all to true)
                }
            }
        }

        return FilecoinCBOR.getCBORData(ret_acc);
    }

    /* Filecoin internal call helpers to enable isMiner check */

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
        FilecoinCBOR.CBORBuffer memory buf = FilecoinCBOR.createCBOR(8);
        FilecoinCBOR.writeUInt64(buf, minerID);
        bytes memory rawRequest = FilecoinCBOR.getCBORData(buf);
        (int256 exit,) = FilecoinCBOR.callById(POWER_ACTOR_ID, MINER_RAW_POWER_METHOD_NUMBER, FilecoinCBOR.CBOR_CODEC, rawRequest, 0, false);
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
}
