// SPDX-License-Identifier: MIT
pragma solidity 0.8.25;

import "./FilecoinCBOR.sol";

// On-chain contract that validates sector status and queries sector expiration
// via the miner actor. GenerateSectorLocation is intended to be called off-chain
// (e.g. via eth_call) and the resulting aux_data is passed in here.
contract SectorStatusChecker {
    // FRC-42 method numbers for miner actor
    uint64 constant VALIDATE_SECTOR_STATUS = 3092458564;
    uint64 constant GET_NOMINAL_SECTOR_EXPIRATION = 3010055991;

    // Storage for last results (queryable from tests)
    bool public lastValid;
    int64 public lastExpiration;

    /// @notice Call ValidateSectorStatus on a miner actor.
    /// status and auxData are obtained off-chain via GenerateSectorLocation.
    function validateSectorStatus(
        uint64 minerActorId,
        uint64 sectorNumber,
        string memory status,
        bytes memory auxData
    ) public returns (bool valid) {
        // CBOR encode ValidateSectorStatusParams: array(3) [ uint64, text, bytes ]
        bytes memory statusBytes = bytes(status);
        FilecoinCBOR.CBORBuffer memory buf = FilecoinCBOR.createCBOR(64 + statusBytes.length + auxData.length);
        FilecoinCBOR.startFixedArray(buf, 3);
        FilecoinCBOR.writeUInt64(buf, sectorNumber);
        FilecoinCBOR.writeTextString(buf, status);
        FilecoinCBOR.writeByteString(buf, auxData);

        (int256 exit, bytes memory ret) = FilecoinCBOR.callById(
            minerActorId, VALIDATE_SECTOR_STATUS, FilecoinCBOR.CBOR_CODEC, FilecoinCBOR.getCBORData(buf), 0, true
        );
        require(exit == 0, "ValidateSectorStatus failed");

        // CBOR decode ValidateSectorStatusReturn (serde transparent): bool
        uint byteIdx = 0;
        (valid, byteIdx) = FilecoinCBOR.readBool(ret, byteIdx);
        lastValid = valid;
    }

    /// @notice Call GetNominalSectorExpiration on a miner actor
    function getNominalSectorExpiration(uint64 minerActorId, uint64 sectorNumber)
        public returns (int64 expiration)
    {
        // CBOR encode SectorNumber (serde transparent): uint64
        FilecoinCBOR.CBORBuffer memory buf = FilecoinCBOR.createCBOR(16);
        FilecoinCBOR.writeUInt64(buf, sectorNumber);

        (int256 exit, bytes memory ret) = FilecoinCBOR.callById(
            minerActorId, GET_NOMINAL_SECTOR_EXPIRATION, FilecoinCBOR.CBOR_CODEC, FilecoinCBOR.getCBORData(buf), 0, true
        );
        require(exit == 0, "GetNominalSectorExpiration failed");

        // CBOR decode GetNominalSectorExpirationReturn (serde transparent): int64
        uint byteIdx = 0;
        (expiration, byteIdx) = FilecoinCBOR.readInt64(ret, byteIdx);
        lastExpiration = expiration;
    }
}
