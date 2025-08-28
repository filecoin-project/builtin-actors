// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

import "./FEVM.sol";

contract NotificationReceiver {
    // Events to track notifications
    event SectorContentChanged(uint64 indexed method, bytes params);
    event NotificationReceived(uint64 indexed sector, uint64 indexed minimumCommitmentEpoch, bytes32 indexed dataCid);
    
    // State variables to track received notifications
    struct SectorNotification {
        uint64 sector;
        uint64 minimumCommitmentEpoch;
        bytes32 dataCid;
        uint256 pieceSize;
        bytes payload;
        uint256 timestamp;
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
     * @dev Toggle whether to reject notifications (for testing)
     */
    function setRejectNotifications(bool _reject) public {
        shouldRejectNotifications = _reject;
    }
    
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
        uint64 minimumCommitmentEpoch,
        bytes32 dataCid,
        uint256 pieceSize,
        bytes memory payload,
        uint256 timestamp
    ) {
        require(index < notifications.length, "Invalid notification index");
        SectorNotification memory notif = notifications[index];
        return (
            notif.sector,
            notif.minimumCommitmentEpoch,
            notif.dataCid,
            notif.pieceSize,
            notif.payload,
            notif.timestamp
        );
    }
    
    /**
     * @dev Handle incoming Filecoin method calls
     * This is the main entry point for receiving notifications from the miner actor
     */
    function handle_filecoin_method(uint64 method, uint64, bytes memory params) public returns (bytes memory) {
        emit SectorContentChanged(method, params);
        
        // Check if this is a sector content changed notification
        if (method == SECTOR_CONTENT_CHANGED) {
            return processSectorContentChanged(params);
        }
        
        // For other methods, just acknowledge receipt
        return abi.encode(true);
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
        // In a real implementation, we would decode CBOR here
        // For testing, we'll process the raw bytes and extract key information
        
        // Check if we should reject this notification
        if (shouldRejectNotifications) {
            // Return a rejection response
            return encodeRejectionResponse();
        }
        
        // For this test contract, we'll store a simplified version of the notification
        // In production, you would properly decode the CBOR data
        
        // Extract some basic info from the params (simplified for testing)
        uint64 sector = extractSectorNumber(params);
        uint64 minimumCommitmentEpoch = extractMinimumCommitmentEpoch(params);
        bytes32 dataCid = extractDataCid(params);
        uint256 pieceSize = extractPieceSize(params);
        bytes memory payload = extractPayload(params);
        
        // Store the notification
        uint256 notificationIndex = notifications.length;
        notifications.push(SectorNotification({
            sector: sector,
            minimumCommitmentEpoch: minimumCommitmentEpoch,
            dataCid: dataCid,
            pieceSize: pieceSize,
            payload: payload,
            timestamp: block.timestamp
        }));
        
        sectorNotificationIndices[sector].push(notificationIndex);
        totalNotifications++;
        
        emit NotificationReceived(sector, minimumCommitmentEpoch, dataCid);
        
        // Return acceptance response
        return encodeAcceptanceResponse();
    }
    
    /**
     * @dev Extract sector number from params (simplified for testing)
     */
    function extractSectorNumber(bytes memory params) internal pure returns (uint64) {
        // In a real implementation, this would properly decode CBOR
        // For testing, return a dummy value or extract from known position
        if (params.length >= 8) {
            return uint64(uint8(params[7])) | 
                   (uint64(uint8(params[6])) << 8) |
                   (uint64(uint8(params[5])) << 16) |
                   (uint64(uint8(params[4])) << 24);
        }
        return 0;
    }
    
    /**
     * @dev Extract minimum commitment epoch from params (simplified)
     */
    function extractMinimumCommitmentEpoch(bytes memory params) internal pure returns (uint64) {
        // Simplified extraction for testing
        if (params.length >= 16) {
            return uint64(uint8(params[15])) | 
                   (uint64(uint8(params[14])) << 8) |
                   (uint64(uint8(params[13])) << 16) |
                   (uint64(uint8(params[12])) << 24);
        }
        return 0;
    }
    
    /**
     * @dev Extract data CID from params (simplified)
     */
    function extractDataCid(bytes memory params) internal pure returns (bytes32) {
        // Simplified extraction for testing
        if (params.length >= 48) {
            bytes32 cid;
            assembly {
                cid := mload(add(params, 48))
            }
            return cid;
        }
        return bytes32(0);
    }
    
    /**
     * @dev Extract piece size from params (simplified)
     */
    function extractPieceSize(bytes memory params) internal pure returns (uint256) {
        // Simplified extraction for testing
        if (params.length >= 24) {
            return uint256(uint64(uint8(params[23])) | 
                          (uint64(uint8(params[22])) << 8) |
                          (uint64(uint8(params[21])) << 16) |
                          (uint64(uint8(params[20])) << 24));
        }
        return 0;
    }
    
    /**
     * @dev Extract payload from params (simplified)
     */
    function extractPayload(bytes memory params) internal pure returns (bytes memory) {
        // For testing, return a portion of the params as payload
        if (params.length > 64) {
            bytes memory payload = new bytes(params.length - 64);
            for (uint i = 0; i < payload.length; i++) {
                payload[i] = params[i + 64];
            }
            return payload;
        }
        return "";
    }
    
    /**
     * @dev Encode an acceptance response for the notification
     * The response should match SectorContentChangedReturn structure
     */
    function encodeAcceptanceResponse() internal pure returns (bytes memory) {
        // Return a properly formatted response indicating acceptance
        // Structure: { sectors: [{ added: [{ accepted: true }] }] }
        // For simplified testing, return a basic acceptance
        return abi.encode(true);
    }
    
    /**
     * @dev Encode a rejection response for the notification
     */
    function encodeRejectionResponse() internal pure returns (bytes memory) {
        // Return a properly formatted response indicating rejection
        return abi.encode(false);
    }
    
    /**
     * @dev Fallback function to handle direct calls
     */
    fallback() external payable {
        // Check if this is a handle_filecoin_method call
        if (msg.data.length >= 4 && bytes4(msg.data[0:4]) == NATIVE_METHOD_SELECTOR) {
            // Decode the parameters
            (uint64 method, uint64 codec, bytes memory params) = abi.decode(msg.data[4:], (uint64, uint64, bytes));
            bytes memory result = handle_filecoin_method(method, codec, params);
            
            // Return the result
            assembly {
                return(add(result, 0x20), mload(result))
            }
        }
    }
    
    receive() external payable {}
}