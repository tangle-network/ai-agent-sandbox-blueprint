// SPDX-License-Identifier: UNLICENSE
pragma solidity ^0.8.26;

import "forge-std/Script.sol";
import "tnt-core/libraries/Types.sol";

/// @notice Minimal interface for requesting + approving a service on blueprint 4.
///         Mirrors the proven service-3 creation flow (deployer requests, operator approves).
interface ITangleSvc {
    function requestService(
        uint64 blueprintId,
        address[] calldata operators,
        bytes calldata config,
        address[] calldata permittedCallers,
        uint64 ttl,
        address paymentToken,
        uint256 paymentAmount,
        Types.ConfidentialityPolicy confidentiality
    )
        external
        payable
        returns (uint64 requestId);

    function approveService(Types.ApprovalParams calldata p) external;

    function serviceCount() external view returns (uint64);
    function serviceRequestCount() external view returns (uint64);
    function isServiceOperator(uint64 serviceId, address op) external view returns (bool);
}

/// @title NewSandboxService
/// @notice Requests + approves a fresh service on blueprint 4 so the manager spawns
///         it via a live ServiceActivated event (no restart, no svc-4-1 re-scan).
///         The new service becomes the PathUSD-e2e target.
contract NewSandboxService is Script {
    // Overridable so the fresh 0.19-BSM blueprint (id 8 on Tempo) can be used.
    uint64 immutable BLUEPRINT_ID = uint64(vm.envOr("BLUEPRINT_ID", uint256(4)));

    function run() external {
        address tangle = vm.envAddress("TANGLE_CONTRACT");
        uint256 ownerKey = vm.envUint("OWNER_KEY");
        uint256 operatorKey = vm.envUint("OPERATOR_KEY");
        address owner = vm.addr(ownerKey);
        address operator = vm.addr(operatorKey);

        ITangleSvc t = ITangleSvc(tangle);

        uint64 svcCountBefore = t.serviceCount();
        console.log("service count BEFORE:", svcCountBefore);
        console.log("new service id will be:", svcCountBefore);

        // 1. Deployer (owner/requester) requests the service.
        address[] memory operators = new address[](1);
        operators[0] = operator;
        address[] memory permitted = new address[](1);
        permitted[0] = owner;

        vm.startBroadcast(ownerKey);
        uint64 reqId = t.requestService(
            BLUEPRINT_ID,
            operators,
            "", // empty config: Cloud-mode decodes SelectionRequest, count 0 -> defaults to operators.length
            permitted,
            0, // ttl 0 = no expiry
            address(0),
            0,
            Types.ConfidentialityPolicy.Any
        );
        vm.stopBroadcast();
        console.log("requestService reqId:", reqId);

        // 2. Operator approves. Empty securityCommitments OK — request has only the
        //    default-TNT requirement, auto-filled at min. No BLS, no TEE.
        Types.ApprovalParams memory p;
        p.requestId = reqId;
        p.securityCommitments = new Types.AssetSecurityCommitment[](0);
        p.blsPubkey = [uint256(0), 0, 0, 0];
        p.blsPopSignature = [uint256(0), 0];
        p.teeCommitments = new Types.TeeAttestationCommitment[](0);

        vm.startBroadcast(operatorKey);
        t.approveService(p);
        vm.stopBroadcast();

        uint64 svcCountAfter = t.serviceCount();
        console.log("service count AFTER:", svcCountAfter);
        uint64 newServiceId = svcCountAfter - 1;
        console.log("NEW SERVICE ID:", newServiceId);
        bool isOp = t.isServiceOperator(newServiceId, operator);
        console.log("operator is service operator:", isOp);
        require(svcCountAfter == svcCountBefore + 1, "service not created");
        require(isOp, "operator not on new service");
    }
}
